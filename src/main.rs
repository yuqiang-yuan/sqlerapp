use std::path::PathBuf;

use gpui_kit::assets::Assets;
use gpui_kit::component::menu::*;
use gpui_kit::component::*;
use gpui_kit::*;
use rust_embed::Embed;
use sqlerapp::actions::{About, Open, Quit, Save};
use sqlerapp::frame::FrameView;

pub struct MyApp {
    menu_bar: Entity<AppMenuBar>,
    current_file: Option<PathBuf>,
}

impl MyApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(target_os = "macos")]
        {
            cx.set_menus(build_menus());
        }

        #[cfg(not(target_os = "macos"))]
        {
            let menus = build_menus()
                .into_iter()
                .map(|menu| menu.owned())
                .collect();
            GlobalState::global_mut(cx).set_app_menus(menus);
        }

        Self {
            menu_bar: AppMenuBar::new(cx),
            current_file: None,
        }
    }
}

impl Render for MyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("main-window")
            .size_full()
            .v_flex()
            .child(
                TitleBar::new()
                    .child(self.menu_bar.clone())
            )
    }
}



#[derive(Embed)]
#[folder = "assets"]
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(file) = AppAssets::get(path) {
            return Ok(Some(file.data));
        }

        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut files = AppAssets::iter()
            .filter(|f| f.starts_with(path))
            .map(SharedString::from)
            .collect::<Vec<_>>();
        files.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(files)
    }
}

fn build_menus() -> Vec<Menu> {
    vec![
        #[cfg(target_os = "macos")]
        {
            Menu {
                name: "SQLER".into(),
                items: vec![
                    MenuItem::action("About", About),
                    MenuItem::separator(),
                    MenuItem::action("Quit", Quit),
                ],
                disabled: false,
            }
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("Open", Open),
                MenuItem::action("Save", Save),
            ],
            disabled: false,
        },
    ]
}

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
        let view = cx.new(|cx| MyApp::new(window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })
    .expect("Failed to open window");

    cx.activate(true);
}

fn main() {
    let app = gpui_kit::application().with_assets(AppAssets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_app_identity("top.sqler.app", "SQLER");

        let window_bounds = WindowBounds::centered(size(px(800.0), px(600.0)), cx);

        cx.spawn(async move |cx| {
            cx.update(|cx| {
                open_main_window(Some(window_bounds), cx);
            });
        })
        .detach();
    });
}
