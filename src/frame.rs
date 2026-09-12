use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use gpui_kit::component::*;

use crate::actions::*;

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

pub struct FrameView {
    menu_bar: Entity<AppMenuBar>,
}

impl FrameView {
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
        }
    }
}

impl Render for FrameView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .size_full()
            .child(
                TitleBar::new()
                    .when(cfg!(not(target_os = "macos")), |tb| tb.child(self.menu_bar.clone()))
            )
    }
}
