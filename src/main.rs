use gpui_kit::assets::Assets;
use gpui_kit::component::*;
use gpui_kit::*;
use sqlerapp::frame::{AppState, FrameView};
use sqlerapp::settings::AppSettings;

fn open_main_window(window_bounds: Option<WindowBounds>, cx: &mut App) {
    // Fall back to a centered default when no geometry was saved.
    let window_bounds =
        window_bounds.unwrap_or_else(|| WindowBounds::centered(size(px(800.), px(600.)), cx));

    let options = WindowOptions {
        window_bounds: Some(window_bounds),
        kind: WindowKind::Normal,
        #[cfg(target_os = "linux")]
        window_decorations: Some(WindowDecorations::Client),
        ..TitleBar::window_options()
    };

    cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| FrameView::new(window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })
    .expect("Failed to open window");
}

fn main() {
    let app = gpui_kit::application().with_assets(Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);

        let settings = AppSettings::load();

        // Apply the persisted theme before any window opens, so the first
        // frame is already in the right mode.
        Theme::change(settings.theme_mode, None, cx);

        // Restore the saved window geometry (if any) for the main window.
        let window_bounds = settings.window_bounds(cx);

        // Snapshot settings on quit. On `LastWindowClosed` platforms (all
        // non-macOS) the window is removed from `cx.windows()` *before*
        // `on_app_quit` fires, so read the geometry from the snapshot kept
        // fresh in `AppState` during render rather than the window list.
        let quit_subscription = cx.on_app_quit(|cx| {
            let window_state = cx.global::<AppState>().last_window_state;
            let theme_mode = cx.theme().mode;

            async move {
                let mut settings = AppSettings::default();
                settings.window = window_state;
                settings.theme_mode = theme_mode;
                if let Err(err) = settings.save() {
                    eprintln!("failed to save settings: {err}");
                }
            }
        });
        // The quit handler must stay registered for the app's lifetime;
        // `on_app_quit` returns a `Subscription` that unregisters on drop.
        std::mem::forget(quit_subscription);

        cx.spawn(async move |cx| {
            cx.update(|cx| open_main_window(window_bounds, cx));
        })
        .detach();
    });
}
