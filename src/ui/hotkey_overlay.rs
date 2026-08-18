use std::cell::RefCell;
use std::cmp::max;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::iter::zip;
use std::rc::Rc;

use niri_config::{Action, Bind, Config, Key, ModKey, Modifiers, Orientation, Trigger};
use pangocairo::cairo::{self, ImageSurface};
use pangocairo::pango::{AttrColor, AttrInt, AttrList, AttrString, FontDescription, Weight};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::input::keyboard::xkb::keysym_get_name;
use smithay::output::{Output, WeakOutput};
use smithay::reexports::gbm::Format as Fourcc;
use smithay::utils::{Scale, Transform};

use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::{output_size, to_physical_precise_round};

const PADDING: i32 = 8;
// const MARGIN: i32 = PADDING * 2;
const FONT: &str = "sans 14px";
const BORDER: i32 = 4;
const LINE_INTERVAL: i32 = 2;
const TITLE: &str = "Important Hotkeys";

pub struct HotkeyOverlay {
    is_open: bool,
    config: Rc<RefCell<Config>>,
    mod_key: ModKey,
    buffers: RefCell<HashMap<WeakOutput, RenderedOverlay>>,
}

pub struct RenderedOverlay {
    buffer: Option<TextureBuffer<GlesTexture>>,
    orientation: Orientation,
}

impl HotkeyOverlay {
    pub fn new(config: Rc<RefCell<Config>>, mod_key: ModKey) -> Self {
        Self {
            is_open: false,
            config,
            mod_key,
            buffers: RefCell::new(HashMap::new()),
        }
    }

    pub fn show(&mut self) -> bool {
        if !self.is_open {
            self.is_open = true;
            true
        } else {
            false
        }
    }

    pub fn hide(&mut self) -> bool {
        if self.is_open {
            self.is_open = false;
            true
        } else {
            false
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn on_hotkey_config_updated(&mut self, mod_key: ModKey) {
        self.mod_key = mod_key;
        self.buffers.borrow_mut().clear();
    }

    /// Renders the overlay for `output`.
    ///
    /// `orientation` selects which spelling the curated list of important hotkeys uses for its
    /// directional entries (see [`collect_actions`]); pass the orientation of `output` itself,
    /// matching [`crate::layout::monitor::Monitor::orientation`] (the same source
    /// `overview_axis()` uses), since the overlay is rendered once per output.
    pub fn render<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        output: &Output,
        orientation: Orientation,
    ) -> Option<PrimaryGpuTextureRenderElement> {
        if !self.is_open {
            return None;
        }

        let scale = output.current_scale().fractional_scale();
        let output_size = output_size(output);

        let mut buffers = self.buffers.borrow_mut();
        buffers.retain(|output, _| output.is_alive());

        // FIXME: should probably use the working area rather than view size.
        let weak = output.downgrade();
        if let Some(rendered) = buffers.get(&weak) {
            if let Some(buffer) = &rendered.buffer {
                if buffer.texture_scale() != Scale::from(scale)
                    || rendered.orientation != orientation
                {
                    buffers.remove(&weak);
                }
            }
        }

        let rendered = buffers.entry(weak).or_insert_with(|| {
            let renderer = renderer.as_gles_renderer();
            render(
                renderer,
                &self.config.borrow(),
                self.mod_key,
                scale,
                orientation,
            )
            .unwrap_or_else(|_| RenderedOverlay {
                buffer: None,
                orientation,
            })
        });
        let buffer = rendered.buffer.as_ref()?;

        let size = buffer.logical_size();
        let location = (output_size.to_f64().to_point() - size.to_point()).downscale(2.);
        let mut location = location.to_physical_precise_round(scale).to_logical(scale);
        location.x = f64::max(0., location.x);
        location.y = f64::max(0., location.y);

        let elem = TextureRenderElement::from_texture_buffer(
            buffer.clone(),
            location,
            0.9,
            None,
            None,
            Kind::Unspecified,
        );

        Some(PrimaryGpuTextureRenderElement(elem))
    }

    /// `orientation` picks the curated list's directional spellings; see [`Self::render`].
    pub fn a11y_text(&self, orientation: Orientation) -> String {
        let config = self.config.borrow();
        let actions = collect_actions(&config, orientation);

        let mut buf = String::new();
        writeln!(&mut buf, "{TITLE}").unwrap();

        for action in actions {
            let Some((key, action)) = format_bind(&config.binds.0, action) else {
                continue;
            };

            let key = key.map(|key| key_name(true, self.mod_key, &key));
            let key = key.as_deref().unwrap_or("not bound");

            let action = match pango::parse_markup(&action, '\0') {
                Ok((_attrs, text, _accel)) => text,
                Err(_) => action.into(),
            };

            writeln!(&mut buf, "{key} {action}").unwrap();
        }

        buf
    }
}

fn format_bind(binds: &[Bind], action: &Action) -> Option<(Option<Key>, String)> {
    let mut bind_with_non_null = None;
    let mut bind_with_custom_title = None;
    let mut found_null_title = false;

    for bind in binds {
        if bind.action != *action {
            continue;
        }

        match &bind.hotkey_overlay_title {
            Some(Some(_)) => {
                bind_with_custom_title.get_or_insert(bind);
            }
            Some(None) => {
                found_null_title = true;
            }
            None => {
                bind_with_non_null.get_or_insert(bind);
            }
        }
    }

    if bind_with_custom_title.is_none() && found_null_title {
        return None;
    }

    let mut title = None;
    let key = if let Some(bind) = bind_with_custom_title.or(bind_with_non_null) {
        if let Some(Some(custom)) = &bind.hotkey_overlay_title {
            title = Some(custom.clone());
        }

        Some(bind.key)
    } else {
        None
    };
    let title = title.unwrap_or_else(|| action_name(action));

    Some((key, title))
}

/// Builds the curated list of important hotkeys shown by the overlay.
///
/// The directional entries below follow `orientation`, per the spatial-twin table established
/// for the rest of the layout: the twin preserves the resolved `AxisDirection`
/// (`crate::layout::axis::AxisDirection`), not the word. For the group family (main axis; see
/// `AxisMap::main_direction`), left/up are both `Backward` and right/down are both `Forward`, so
/// group left <-> up and right <-> down. `ConsumeOrExpelWindow{Left,Right,Up,Down}` follows the
/// group twin too, since it moves a window along the strip like a group move. For the workspace
/// family (cross axis; see `AxisMap::cross_direction`), down/right are both `Forward` and
/// up/left are both `Backward`, so workspace down <-> right and up <-> left. On
/// [`Orientation::Horizontal`] this reproduces the original, orientation-blind list.
///
/// The dimension family also follows `orientation`: `SwitchPresetGroupWidth` (horizontal) and
/// `SwitchPresetGroupHeight` (vertical) both name the group's strip-span dimension, so the
/// curated slot shows whichever one is live for the current orientation.
///
/// `orientation` is the single source for all of these, rather than each family's own dispatch
/// source (workspace axis for group/window actions, monitor axis for workspace stacking): see
/// [`HotkeyOverlay::render`].
fn collect_actions(config: &Config, orientation: Orientation) -> Vec<&Action> {
    let binds = &config.binds.0;
    let vertical = matches!(orientation, Orientation::Vertical);

    // Collect actions that we want to show.
    let mut actions = vec![&Action::ShowHotkeyOverlay];

    // Prefer Quit(false) if found, otherwise try Quit(true), and if there's neither, fall back to
    // Quit(false).
    if binds.iter().any(|bind| bind.action == Action::Quit(false)) {
        actions.push(&Action::Quit(false));
    } else if binds.iter().any(|bind| bind.action == Action::Quit(true)) {
        actions.push(&Action::Quit(true));
    } else {
        actions.push(&Action::Quit(false));
    }

    actions.push(&Action::CloseWindow);

    if vertical {
        actions.extend(&[
            &Action::FocusGroupUp,
            &Action::FocusGroupDown,
            &Action::MoveGroupUp,
            &Action::MoveGroupDown,
            &Action::FocusWorkspaceRight,
            &Action::FocusWorkspaceLeft,
        ]);
    } else {
        actions.extend(&[
            &Action::FocusGroupLeft,
            &Action::FocusGroupRight,
            &Action::MoveGroupLeft,
            &Action::MoveGroupRight,
            &Action::FocusWorkspaceDown,
            &Action::FocusWorkspaceUp,
        ]);
    }

    if vertical {
        // The twin of -down (Forward) is -right, not -left; see the doc comment above.
        // Prefer move-group-to-workspace-right, but fall back to move-window-to-workspace-right.
        if let Some(bind) = binds
            .iter()
            .find(|bind| matches!(bind.action, Action::MoveGroupToWorkspaceRight(_)))
        {
            actions.push(&bind.action);
        } else if binds
            .iter()
            .any(|bind| matches!(bind.action, Action::MoveWindowToWorkspaceRight(_)))
        {
            actions.push(&Action::MoveWindowToWorkspaceRight(true));
        } else {
            actions.push(&Action::MoveGroupToWorkspaceRight(true));
        }

        // Same for -left (the twin of -up).
        if let Some(bind) = binds
            .iter()
            .find(|bind| matches!(bind.action, Action::MoveGroupToWorkspaceLeft(_)))
        {
            actions.push(&bind.action);
        } else if binds
            .iter()
            .any(|bind| matches!(bind.action, Action::MoveWindowToWorkspaceLeft(_)))
        {
            actions.push(&Action::MoveWindowToWorkspaceLeft(true));
        } else {
            actions.push(&Action::MoveGroupToWorkspaceLeft(true));
        }
    } else {
        // Prefer move-column-to-workspace-down, but fall back to move-window-to-workspace-down.
        if let Some(bind) = binds
            .iter()
            .find(|bind| matches!(bind.action, Action::MoveGroupToWorkspaceDown(_)))
        {
            actions.push(&bind.action);
        } else if binds
            .iter()
            .any(|bind| matches!(bind.action, Action::MoveWindowToWorkspaceDown(_)))
        {
            actions.push(&Action::MoveWindowToWorkspaceDown(true));
        } else {
            actions.push(&Action::MoveGroupToWorkspaceDown(true));
        }

        // Same for -up.
        if let Some(bind) = binds
            .iter()
            .find(|bind| matches!(bind.action, Action::MoveGroupToWorkspaceUp(_)))
        {
            actions.push(&bind.action);
        } else if binds
            .iter()
            .any(|bind| matches!(bind.action, Action::MoveWindowToWorkspaceUp(_)))
        {
            actions.push(&Action::MoveWindowToWorkspaceUp(true));
        } else {
            actions.push(&Action::MoveGroupToWorkspaceUp(true));
        }
    }

    if vertical {
        actions.push(&Action::SwitchPresetGroupHeight);
    } else {
        actions.push(&Action::SwitchPresetGroupWidth);
    }
    actions.push(&Action::MaximizeGroup);

    if vertical {
        actions.extend(&[
            &Action::ConsumeOrExpelWindowUp,
            &Action::ConsumeOrExpelWindowDown,
        ]);
    } else {
        actions.extend(&[
            &Action::ConsumeOrExpelWindowLeft,
            &Action::ConsumeOrExpelWindowRight,
        ]);
    }

    actions.extend(&[
        &Action::ToggleWindowFloating,
        &Action::SwitchFocusBetweenFloatingAndTiling,
        &Action::ToggleOverview,
    ]);

    // Screenshot is not as important, can omit if not bound.
    if let Some(bind) = binds
        .iter()
        .find(|bind| matches!(bind.action, Action::Screenshot(_, _)))
    {
        actions.push(&bind.action);
    }

    // Add actions with a custom hotkey-overlay-title.
    for bind in binds {
        if matches!(bind.hotkey_overlay_title, Some(Some(_))) {
            // Avoid duplicate actions.
            if !actions.contains(&&bind.action) {
                actions.push(&bind.action);
            }
        }
    }

    // Add the spawn actions.
    for bind in binds.iter().filter(|bind| {
        matches!(bind.action, Action::Spawn(_) | Action::SpawnSh(_))
            // Only show binds with Mod or Super to filter out stuff like volume up/down.
            && (bind.key.modifiers.contains(Modifiers::COMPOSITOR)
                || bind.key.modifiers.contains(Modifiers::SUPER))
            // Also filter out wheel and touchpad scroll binds.
            && matches!(bind.key.trigger, Trigger::Keysym(_))
    }) {
        let action = &bind.action;

        // We only show one bind for each action, so we need to deduplicate the Spawn actions.
        if !actions.contains(&action) {
            actions.push(action);
        }
    }

    if config.hotkey_overlay.hide_not_bound {
        // Only keep actions that have been bound
        actions.retain(|&action| binds.iter().any(|bind| bind.action == *action))
    }

    actions
}

fn render(
    renderer: &mut GlesRenderer,
    config: &Config,
    mod_key: ModKey,
    scale: f64,
    orientation: Orientation,
) -> anyhow::Result<RenderedOverlay> {
    let _span = tracy_client::span!("hotkey_overlay::render");

    // let margin = MARGIN * scale;
    let padding: i32 = to_physical_precise_round(scale, PADDING);
    let line_interval: i32 = to_physical_precise_round(scale, LINE_INTERVAL);

    // FIXME: if it doesn't fit, try splitting in two columns or something.
    // let mut target_size = output_size;
    // target_size.w -= margin * 2;
    // target_size.h -= margin * 2;
    // anyhow::ensure!(target_size.w > 0 && target_size.h > 0);

    let strings = collect_actions(config, orientation)
        .into_iter()
        .filter_map(|action| format_bind(&config.binds.0, action))
        .map(|(key, action)| {
            let key = key.map(|key| key_name(false, mod_key, &key));
            let key = key.as_deref().unwrap_or("(not bound)");
            let key = format!(" {key} ");
            (key, action)
        })
        .collect::<Vec<_>>();

    let mut font = FontDescription::from_string(FONT);
    font.set_absolute_size(to_physical_precise_round(scale, font.size()));

    let surface = ImageSurface::create(cairo::Format::ARgb32, 0, 0)?;
    let cr = cairo::Context::new(&surface)?;
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));

    let bold = AttrList::new();
    bold.insert(AttrInt::new_weight(Weight::Bold));
    layout.set_attributes(Some(&bold));
    layout.set_text(TITLE);
    let title_size = layout.pixel_size();

    let attrs = AttrList::new();
    attrs.insert(AttrString::new_family("Monospace"));
    attrs.insert(AttrColor::new_background(12000, 12000, 12000));

    layout.set_attributes(Some(&attrs));
    let key_sizes = strings
        .iter()
        .map(|(key, _)| {
            layout.set_text(key);
            layout.pixel_size()
        })
        .collect::<Vec<_>>();

    layout.set_attributes(None);
    let action_sizes = strings
        .iter()
        .map(|(_, action)| {
            layout.set_markup(action);
            layout.pixel_size()
        })
        .collect::<Vec<_>>();

    let key_width = key_sizes.iter().map(|(w, _)| w).max().unwrap();
    let action_width = action_sizes.iter().map(|(w, _)| w).max().unwrap();
    let mut width = key_width + padding + action_width;

    let mut height = zip(&key_sizes, &action_sizes)
        .map(|((_, key_h), (_, act_h))| max(key_h, act_h))
        .sum::<i32>()
        + (key_sizes.len() - 1) as i32 * line_interval
        + title_size.1
        + padding;

    width += padding * 2;
    height += padding * 2;

    let surface = ImageSurface::create(cairo::Format::ARgb32, width, height)?;
    let cr = cairo::Context::new(&surface)?;
    cr.set_source_rgb(0.1, 0.1, 0.1);
    cr.paint()?;

    cr.move_to(padding.into(), padding.into());
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));

    cr.set_source_rgb(1., 1., 1.);

    cr.move_to(((width - title_size.0) / 2).into(), padding.into());
    layout.set_attributes(Some(&bold));
    layout.set_text(TITLE);
    pangocairo::functions::show_layout(&cr, &layout);

    cr.move_to(padding.into(), (padding + title_size.1 + padding).into());

    for ((key, action), ((_, key_h), (_, act_h))) in zip(&strings, zip(&key_sizes, &action_sizes)) {
        layout.set_attributes(Some(&attrs));
        layout.set_text(key);
        pangocairo::functions::show_layout(&cr, &layout);

        cr.rel_move_to((key_width + padding).into(), 0.);

        let (attrs, text) = match pango::parse_markup(action, '\0') {
            Ok((attrs, text, _accel)) => (Some(attrs), text),
            Err(err) => {
                warn!("error parsing markup for key {key}: {err}");
                (None, action.into())
            }
        };

        layout.set_attributes(attrs.as_ref());
        layout.set_text(&text);
        pangocairo::functions::show_layout(&cr, &layout);

        cr.rel_move_to(
            (-(key_width + padding)).into(),
            (max(key_h, act_h) + line_interval).into(),
        );
    }

    cr.move_to(0., 0.);
    cr.line_to(width.into(), 0.);
    cr.line_to(width.into(), height.into());
    cr.line_to(0., height.into());
    cr.line_to(0., 0.);
    cr.set_source_rgb(0.5, 0.8, 1.0);
    // Keep the border width even to avoid blurry edges.
    cr.set_line_width((f64::from(BORDER) / 2. * scale).round() * 2.);
    cr.stroke()?;
    drop(cr);

    let data = surface.take_data().unwrap();
    let buffer = TextureBuffer::from_memory(
        renderer,
        &data,
        Fourcc::Argb8888,
        (width, height),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )?;

    Ok(RenderedOverlay {
        buffer: Some(buffer),
        orientation,
    })
}

fn action_name(action: &Action) -> String {
    match action {
        Action::Quit(_) => String::from("Exit niri"),
        Action::ShowHotkeyOverlay => String::from("Show Important Hotkeys"),
        Action::CloseWindow => String::from("Close Focused Window"),
        Action::FocusGroupLeft => String::from("Focus Group to the Left"),
        Action::FocusGroupRight => String::from("Focus Group to the Right"),
        Action::FocusGroupUp => String::from("Focus Group Up"),
        Action::FocusGroupDown => String::from("Focus Group Down"),
        Action::MoveGroupLeft => String::from("Move Group Left"),
        Action::MoveGroupRight => String::from("Move Group Right"),
        Action::MoveGroupUp => String::from("Move Group Up"),
        Action::MoveGroupDown => String::from("Move Group Down"),
        Action::FocusWorkspaceDown => String::from("Switch Workspace Down"),
        Action::FocusWorkspaceUp => String::from("Switch Workspace Up"),
        Action::FocusWorkspaceLeft => String::from("Switch Workspace Left"),
        Action::FocusWorkspaceRight => String::from("Switch Workspace Right"),
        Action::MoveGroupToWorkspaceDown(_) => String::from("Move Group to Workspace Down"),
        Action::MoveGroupToWorkspaceUp(_) => String::from("Move Group to Workspace Up"),
        Action::MoveGroupToWorkspaceLeft(_) => String::from("Move Group to Workspace Left"),
        Action::MoveGroupToWorkspaceRight(_) => String::from("Move Group to Workspace Right"),
        Action::MoveWindowToWorkspaceDown(_) => String::from("Move Window to Workspace Down"),
        Action::MoveWindowToWorkspaceUp(_) => String::from("Move Window to Workspace Up"),
        Action::MoveWindowToWorkspaceLeft(_) => String::from("Move Window to Workspace Left"),
        Action::MoveWindowToWorkspaceRight(_) => String::from("Move Window to Workspace Right"),
        Action::SwitchPresetGroupWidth => String::from("Switch Preset Group Widths"),
        Action::SwitchPresetGroupHeight => String::from("Switch Preset Group Heights"),
        Action::MaximizeGroup => String::from("Maximize Group"),
        Action::ConsumeOrExpelWindowLeft => String::from("Consume or Expel Window Left"),
        Action::ConsumeOrExpelWindowRight => String::from("Consume or Expel Window Right"),
        Action::ConsumeOrExpelWindowUp => String::from("Consume or Expel Window Up"),
        Action::ConsumeOrExpelWindowDown => String::from("Consume or Expel Window Down"),
        Action::ToggleWindowFloating => String::from("Move Window Between Floating and Tiling"),
        Action::SwitchFocusBetweenFloatingAndTiling => {
            String::from("Switch Focus Between Floating and Tiling")
        }
        Action::ToggleOverview => String::from("Open the Overview"),
        Action::Screenshot(_, _) => String::from("Take a Screenshot"),
        Action::Spawn(args) => format!(
            "Spawn <span face='monospace' bgcolor='#000000'>{}</span>",
            args.first().unwrap_or(&String::new())
        ),
        Action::SpawnSh(command) => format!(
            "Spawn <span face='monospace' bgcolor='#000000'>{}</span>",
            // Fairly crude but should get the job done in most cases.
            command.split_ascii_whitespace().next().unwrap_or("")
        ),
        _ => String::from("FIXME: Unknown"),
    }
}

fn key_name(screen_reader: bool, mod_key: ModKey, key: &Key) -> String {
    let mut name = String::new();

    let has_comp_mod = key.modifiers.contains(Modifiers::COMPOSITOR);

    // Compositor mod goes first.
    if has_comp_mod {
        match mod_key {
            ModKey::Super => {
                name.push_str("Super + ");
            }
            ModKey::Alt => {
                name.push_str("Alt + ");
            }
            ModKey::Shift => {
                name.push_str("Shift + ");
            }
            ModKey::Ctrl => {
                name.push_str("Ctrl + ");
            }
            ModKey::IsoLevel3Shift => {
                name.push_str("Mod5 + ");
            }
            ModKey::IsoLevel5Shift => {
                name.push_str("Mod3 + ");
            }
        }
    }

    if key.modifiers.contains(Modifiers::SUPER) && !(has_comp_mod && mod_key == ModKey::Super) {
        name.push_str("Super + ");
    }
    if key.modifiers.contains(Modifiers::CTRL) && !(has_comp_mod && mod_key == ModKey::Ctrl) {
        name.push_str("Ctrl + ");
    }
    if key.modifiers.contains(Modifiers::SHIFT) && !(has_comp_mod && mod_key == ModKey::Shift) {
        name.push_str("Shift + ");
    }
    if key.modifiers.contains(Modifiers::ALT) && !(has_comp_mod && mod_key == ModKey::Alt) {
        name.push_str("Alt + ");
    }
    if key.modifiers.contains(Modifiers::ISO_LEVEL3_SHIFT)
        && !(has_comp_mod && mod_key == ModKey::IsoLevel3Shift)
    {
        name.push_str("Mod5 + ");
    }
    if key.modifiers.contains(Modifiers::ISO_LEVEL5_SHIFT)
        && !(has_comp_mod && mod_key == ModKey::IsoLevel5Shift)
    {
        name.push_str("Mod3 + ");
    }

    let pretty = match key.trigger {
        Trigger::Keysym(keysym) => prettify_keysym_name(screen_reader, &keysym_get_name(keysym)),
        Trigger::MouseLeft => String::from("Mouse Left"),
        Trigger::MouseRight => String::from("Mouse Right"),
        Trigger::MouseMiddle => String::from("Mouse Middle"),
        Trigger::MouseBack => String::from("Mouse Back"),
        Trigger::MouseForward => String::from("Mouse Forward"),
        Trigger::WheelScrollDown => String::from("Wheel Scroll Down"),
        Trigger::WheelScrollUp => String::from("Wheel Scroll Up"),
        Trigger::WheelScrollLeft => String::from("Wheel Scroll Left"),
        Trigger::WheelScrollRight => String::from("Wheel Scroll Right"),
        Trigger::TouchpadScrollDown => String::from("Touchpad Scroll Down"),
        Trigger::TouchpadScrollUp => String::from("Touchpad Scroll Up"),
        Trigger::TouchpadScrollLeft => String::from("Touchpad Scroll Left"),
        Trigger::TouchpadScrollRight => String::from("Touchpad Scroll Right"),
        Trigger::TabletStylusButton1 => String::from("Tablet Stylus Button 1"),
        Trigger::TabletStylusButton2 => String::from("Tablet Stylus Button 2"),
        Trigger::TabletStylusButton3 => String::from("Tablet Stylus Button 3"),
    };
    name.push_str(&pretty);

    name
}

fn prettify_keysym_name(screen_reader: bool, name: &str) -> String {
    let name = if screen_reader {
        name
    } else {
        match name {
            "slash" => "/",
            "comma" => ",",
            "period" => ".",
            "minus" => "-",
            "equal" => "=",
            "grave" => "`",
            "bracketleft" => "[",
            "bracketright" => "]",
            "adiaeresis" => "Ä",
            "ediaeresis" => "Ë",
            "idiaeresis" => "Ï",
            "odiaeresis" => "Ö",
            "udiaeresis" => "Ü",
            "ydiaeresis" => "Ÿ",
            "wdiaeresis" => "Ẅ",
            _ => name,
        }
    };

    let name = match name {
        "Next" => "Page Down",
        "Prior" => "Page Up",
        "Print" => "PrtSc",
        "Return" => "Enter",
        "space" => "Space",
        _ => name,
    };

    if name.len() == 1 && name.is_ascii() {
        name.to_ascii_uppercase()
    } else {
        name.into()
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;

    #[track_caller]
    fn check(config: &str, action: Action) -> String {
        let config = Config::parse_mem(config).unwrap();
        if let Some((key, title)) = format_bind(&config.binds.0, &action) {
            let key = key.map(|key| key_name(false, ModKey::Super, &key));
            let key = key.as_deref().unwrap_or("(not bound)");
            format!(" {key} : {title}")
        } else {
            String::from("None")
        }
    }

    #[test]
    fn test_format_bind() {
        // Not bound.
        assert_snapshot!(check("", Action::Screenshot(true, None)), @" (not bound) : Take a Screenshot");

        // Bound with a default title.
        assert_snapshot!(
            check(
                r#"binds {
                    Mod+P { screenshot; }
                }"#,
                Action::Screenshot(true, None),
            ),
            @" Super + P : Take a Screenshot"
        );

        // Custom title.
        assert_snapshot!(
            check(
                r#"binds {
                    Mod+P hotkey-overlay-title="Hello" { screenshot; }
                }"#,
                Action::Screenshot(true, None),
            ),
            @" Super + P : Hello"
        );

        // Prefer first bind.
        assert_snapshot!(
            check(
                r#"binds {
                    Mod+P { screenshot; }
                    Print { screenshot; }
                }"#,
                Action::Screenshot(true, None),
            ),
            @" Super + P : Take a Screenshot"
        );

        // Prefer bind with custom title.
        assert_snapshot!(
            check(
                r#"binds {
                    Mod+P { screenshot; }
                    Print hotkey-overlay-title="My Cool Bind" { screenshot; }
                }"#,
                Action::Screenshot(true, None),
            ),
            @" PrtSc : My Cool Bind"
        );

        // Prefer first bind with custom title.
        assert_snapshot!(
            check(
                r#"binds {
                    Mod+P hotkey-overlay-title="First" { screenshot; }
                    Print hotkey-overlay-title="My Cool Bind" { screenshot; }
                }"#,
                Action::Screenshot(true, None),
            ),
            @" Super + P : First"
        );

        // Any bind with null title hides it.
        assert_snapshot!(
            check(
                r#"binds {
                    Mod+P { screenshot; }
                    Print hotkey-overlay-title=null { screenshot; }
                }"#,
                Action::Screenshot(true, None),
            ),
            @"None"
        );

        // Custom title takes preference over null.
        assert_snapshot!(
            check(
                r#"binds {
                    Mod+P hotkey-overlay-title="Hello" { screenshot; }
                    Print hotkey-overlay-title=null { screenshot; }
                }"#,
                Action::Screenshot(true, None),
            ),
            @" Super + P : Hello"
        );
    }

    #[test]
    fn collect_actions_follows_orientation() {
        let config = Config::parse_mem("").unwrap();

        let horizontal = collect_actions(&config, Orientation::Horizontal);
        let vertical = collect_actions(&config, Orientation::Vertical);

        // Horizontal reproduces today's list, unaffected by this change.
        assert_eq!(
            horizontal,
            vec![
                &Action::ShowHotkeyOverlay,
                &Action::Quit(false),
                &Action::CloseWindow,
                &Action::FocusGroupLeft,
                &Action::FocusGroupRight,
                &Action::MoveGroupLeft,
                &Action::MoveGroupRight,
                &Action::FocusWorkspaceDown,
                &Action::FocusWorkspaceUp,
                &Action::MoveGroupToWorkspaceDown(true),
                &Action::MoveGroupToWorkspaceUp(true),
                &Action::SwitchPresetGroupWidth,
                &Action::MaximizeGroup,
                &Action::ConsumeOrExpelWindowLeft,
                &Action::ConsumeOrExpelWindowRight,
                &Action::ToggleWindowFloating,
                &Action::SwitchFocusBetweenFloatingAndTiling,
                &Action::ToggleOverview,
            ]
        );

        // Vertical replaces every directional entry with its spatial twin, preserving the
        // resolved AxisDirection rather than the word: group left/right -> up/down (both stay
        // Backward/Forward respectively), and workspace down/up -> right/left (down and right
        // are both Forward, up and left are both Backward). The rest is untouched.
        assert_eq!(
            vertical,
            vec![
                &Action::ShowHotkeyOverlay,
                &Action::Quit(false),
                &Action::CloseWindow,
                &Action::FocusGroupUp,
                &Action::FocusGroupDown,
                &Action::MoveGroupUp,
                &Action::MoveGroupDown,
                &Action::FocusWorkspaceRight,
                &Action::FocusWorkspaceLeft,
                &Action::MoveGroupToWorkspaceRight(true),
                &Action::MoveGroupToWorkspaceLeft(true),
                &Action::SwitchPresetGroupHeight,
                &Action::MaximizeGroup,
                &Action::ConsumeOrExpelWindowUp,
                &Action::ConsumeOrExpelWindowDown,
                &Action::ToggleWindowFloating,
                &Action::SwitchFocusBetweenFloatingAndTiling,
                &Action::ToggleOverview,
            ]
        );

        // None of the horizontal directional entries leak into the vertical list.
        for dead in [
            &Action::FocusGroupLeft,
            &Action::FocusGroupRight,
            &Action::MoveGroupLeft,
            &Action::MoveGroupRight,
            &Action::FocusWorkspaceDown,
            &Action::FocusWorkspaceUp,
            &Action::MoveGroupToWorkspaceDown(true),
            &Action::MoveGroupToWorkspaceUp(true),
            &Action::ConsumeOrExpelWindowLeft,
            &Action::ConsumeOrExpelWindowRight,
            &Action::SwitchPresetGroupWidth,
        ] {
            assert!(
                !vertical.contains(&dead),
                "{dead:?} should not appear in the vertical list"
            );
        }
    }
}
