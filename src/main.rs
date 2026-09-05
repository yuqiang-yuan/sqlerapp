use gpui_kit::assets::Assets;
use gpui_kit::component::*;
use gpui_kit::*;
use sqlerapp::frame::FrameView;

fn open_main_window(cx: &mut App) {
    // Centering needs the display, which is only available on a live App.
    let window_bounds = WindowBounds::centered(size(px(800.), px(600.)), cx);

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

        // The Open/Save action handlers now live on `FrameView` itself
        // (registered via `cx.listener` on the view's element), so they get
        // `&mut self` and can write state directly. See `src/frame.rs`.

        cx.spawn(async move |cx| {
            cx.update(|cx| open_main_window(cx));
        }).detach();
    });
}
