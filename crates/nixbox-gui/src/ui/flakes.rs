//! Flake search: hits on the left, the selected flake's details on the right.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};
use nixbox_config::Target;
use nixbox_nix::flakes::FlakeDetails;

use super::{empty, muted, page_header, section};
use crate::app::NixboxApp;

pub fn render(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
    let header = page_header(
        "Flakes",
        "Find flakes on GitHub and add their package or module to your configuration.",
        None,
        cx,
    );
    let search = div().px_6().pb_3().child(
        Input::new(&app.flake_input)
            .prefix(IconName::Github)
            .cleanable(true),
    );

    let hits = if app.flake_results.is_empty() {
        let text = if app.flake_searching {
            "Searching GitHub..."
        } else {
            "Search by project name or by what the flake contains."
        };
        empty(text, cx).into_any_element()
    } else {
        let (active, hover, border) = (
            cx.theme().list_active,
            cx.theme().list_hover,
            cx.theme().border,
        );
        v_flex()
            .id("flake-hits")
            .size_full()
            .overflow_y_scroll()
            .children(app.flake_results.iter().enumerate().map(|(index, hit)| {
                let selected = app.flake_selected == Some(index);
                let packages = hit.packages.len();
                v_flex()
                    .id(("flake-hit", index))
                    .px_6()
                    .py_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(border)
                    .cursor_pointer()
                    .when(selected, |row| row.bg(active))
                    .hover(move |row| row.bg(hover))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_flake(index, cx)))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(hit.repo.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .when(packages > 0, |tags| {
                                tags.child(
                                    Tag::secondary()
                                        .xsmall()
                                        .child(format!("{packages} package(s)")),
                                )
                            })
                            .when(hit.nixos_module.is_some(), |tags| {
                                tags.child(Tag::secondary().xsmall().child("NixOS module"))
                            })
                            .when(hit.home_manager_module.is_some(), |tags| {
                                tags.child(Tag::secondary().xsmall().child("Home Manager module"))
                            }),
                    )
            }))
            .into_any_element()
    };

    let details = match (&app.flake_details, app.flake_detail_loading) {
        (Some(details), _) => {
            render_details(details, app.session.engine.config.target, cx).into_any_element()
        }
        (None, true) => v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(Spinner::new())
            .into_any_element(),
        (None, false) => empty("Select a flake to see what it provides.", cx).into_any_element(),
    };

    v_flex()
        .size_full()
        .child(header)
        .child(search)
        .child(
            h_flex()
                .flex_1()
                .min_h_0()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(div().w_2_5().h_full().child(hits))
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .child(details),
                ),
        )
        .into_any_element()
}

fn render_details(
    details: &FlakeDetails,
    target: Target,
    cx: &mut Context<NixboxApp>,
) -> impl IntoElement {
    v_flex()
        .id("flake-details")
        .size_full()
        .overflow_y_scroll()
        .pb_4()
        .child(
            h_flex()
                .px_6()
                .pt_4()
                .gap_3()
                .child(
                    v_flex()
                        .flex_1()
                        .gap_1()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(details.repo.clone()),
                        )
                        .child(muted(
                            details
                                .description
                                .clone()
                                .unwrap_or_else(|| "No description.".into()),
                            cx,
                        )),
                )
                .child(
                    Button::new("install-flake")
                        .primary()
                        .icon(IconName::Plus)
                        .label(format!("Install into {}", target.label()))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.install_flake(window, cx)),
                        ),
                ),
        )
        .child(
            h_flex()
                .px_6()
                .pt_2()
                .gap_2()
                .child(
                    Tag::secondary()
                        .xsmall()
                        .child(format!("{} stars", details.stars)),
                )
                .when(details.archived, |tags| {
                    tags.child(Tag::warning().xsmall().child("archived"))
                })
                .children(
                    details
                        .topics
                        .iter()
                        .take(6)
                        .map(|topic| Tag::secondary().outline().xsmall().child(topic.clone())),
                ),
        )
        .child(section("Packages", details.packages.len(), cx))
        .children(details.packages.iter().map(|package| {
            h_flex()
                .px_6()
                .py_1()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(package.attr.clone()),
                )
                .child(muted(format!("{} {}", package.name, package.version), cx))
        }))
        .child(section("Modules", modules(details).len(), cx))
        .children(
            modules(details)
                .into_iter()
                .map(|module| h_flex().px_6().py_1().child(module)),
        )
        .child(section("Inputs", details.inputs.len(), cx))
        .children(
            details
                .inputs
                .iter()
                .map(|input| h_flex().px_6().py_1().child(muted(input.clone(), cx))),
        )
}

fn modules(details: &FlakeDetails) -> Vec<String> {
    let nixos = details
        .nixos_module
        .as_ref()
        .map(|module| format!("nixosModules.{module}"));
    let home = details
        .home_manager_module
        .as_ref()
        .map(|module| format!("homeManagerModules.{module}"));
    nixos.into_iter().chain(home).collect()
}
