//! One installed package's module options: the list on the left, the
//! selected option and its editor on the right.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};
use nixbox_core::options::{choices, format_value, locked_reason, plain_markdown};
use nixbox_nix::options::{OptionEntry, OptionKind};
use nixbox_nix::settings::SettingValue;

use super::{empty, muted, page_header};
use crate::app::NixboxApp;
use crate::options::OptionsView;

pub fn render(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
    let Some(view) = &app.options else {
        return empty("Open a package's options from the Installed page.", cx).into_any_element();
    };
    let staged = view.staged.len();
    let namespaces = view
        .set
        .as_deref()
        .map(|set| {
            set.namespaces
                .iter()
                .map(|path| path.join("."))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let subtitle = if namespaces.is_empty() {
        format!(
            "Module options for {} in {}.",
            view.package,
            view.scope.label()
        )
    } else {
        format!("{namespaces} in {}.", view.scope.label())
    };
    let header = page_header(
        format!("{} options", view.package),
        subtitle,
        Some(
            h_flex()
                .gap_2()
                .child(
                    Button::new("options-back")
                        .ghost()
                        .icon(IconName::ArrowLeft)
                        .label("Back")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_options(window, cx)),
                        ),
                )
                .child(
                    Button::new("options-reload")
                        .ghost()
                        .label("Reload")
                        .disabled(view.loading)
                        .on_click(cx.listener(|this, _, _, cx| this.reload_options(cx))),
                )
                .child(
                    Button::new("options-discard")
                        .outline()
                        .label("Discard")
                        .disabled(staged == 0)
                        .on_click(cx.listener(|this, _, _, cx| this.discard_options(cx))),
                )
                .child(
                    Button::new("options-apply")
                        .primary()
                        .label(if staged == 0 {
                            "Apply".to_string()
                        } else {
                            format!("Apply {staged}")
                        })
                        .disabled(staged == 0)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.apply_options(window, cx)),
                        ),
                )
                .into_any_element(),
        ),
        cx,
    );
    let filter = div().px_6().pb_3().child(
        Input::new(&app.options_filter)
            .prefix(IconName::Search)
            .cleanable(true),
    );

    let body = if view.loading {
        empty("Evaluating your configuration's options...", cx).into_any_element()
    } else if let Some(error) = &view.error {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Could not read options."),
            )
            .child(muted(error.clone(), cx))
            .into_any_element()
    } else if view
        .set
        .as_deref()
        .is_some_and(|set| set.namespaces.is_empty())
    {
        empty(
            format!(
                "No module options found for {}. Only packages with a programs.* or services.* module in your configuration have options to set.",
                view.package
            ),
            cx,
        )
        .into_any_element()
    } else {
        let filter_text = app.options_filter.read(cx).value().to_string();
        h_flex()
            .size_full()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .w_1_2()
                    .h_full()
                    .child(option_list(app, view, &filter_text, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .child(details(app, view, cx)),
            )
            .into_any_element()
    };

    v_flex()
        .size_full()
        .child(header)
        .child(filter)
        .child(div().flex_1().min_h_0().child(body))
        .into_any_element()
}

fn option_list(
    app: &NixboxApp,
    view: &OptionsView,
    filter: &str,
    cx: &mut Context<NixboxApp>,
) -> AnyElement {
    let visible = view.visible(filter);
    if visible.is_empty() {
        return empty("No options match.", cx).into_any_element();
    }
    let (active, hover, border) = (
        cx.theme().list_active,
        cx.theme().list_hover,
        cx.theme().border,
    );
    v_flex()
        .id("options")
        .size_full()
        .overflow_y_scroll()
        .children(visible.into_iter().enumerate().map(|(index, entry)| {
            let path = entry.path.clone();
            let selected = view.selected.as_ref() == Some(&entry.path);
            let staged = view.staged.get(&entry.path);
            let queued = app.session.queued_option(view.scope, &entry.path);
            let value = match (staged, &queued) {
                (Some(value), _) | (None, Some(value)) => setting_text(value.as_ref()),
                (None, None) => format_value(entry.value.as_ref()),
            };
            let locked = locked_reason(entry);
            let tag = if staged.is_some() {
                Some("staged")
            } else if queued.is_some() {
                Some("queued")
            } else if app.session.engine.sets_option(view.scope, entry) {
                Some("nixbox")
            } else {
                None
            };
            h_flex()
                .id(("option", index))
                .px_6()
                .py_2()
                .gap_3()
                .border_b_1()
                .border_color(border)
                .cursor_pointer()
                .when(selected, |row| row.bg(active))
                .hover(move |row| row.bg(hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_option(path.clone(), window, cx);
                }))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .when(locked.is_some(), |name| {
                                    name.text_color(cx.theme().muted_foreground)
                                })
                                .child(view.display_name(entry)),
                        )
                        .child(
                            muted(
                                match &locked {
                                    Some(reason) => format!("{value}, {reason}"),
                                    None => value,
                                },
                                cx,
                            )
                            .truncate(),
                        ),
                )
                .children(tag.map(|tag| Tag::secondary().xsmall().child(tag)))
        }))
        .into_any_element()
}

fn details(app: &NixboxApp, view: &OptionsView, cx: &mut Context<NixboxApp>) -> AnyElement {
    let Some(entry) = view.selected_entry() else {
        return empty("Select an option to see and change it.", cx).into_any_element();
    };
    let locked = locked_reason(entry);
    let ours = app.session.engine.sets_option(view.scope, entry);
    let status = match &locked {
        Some(reason) => format!("Locked, {reason}."),
        None if ours => "Set by nixbox.".into(),
        None => "Editable.".into(),
    };
    let queued = app.session.queued_option(view.scope, &entry.path);

    v_flex()
        .id("option-details")
        .size_full()
        .overflow_y_scroll()
        .px_6()
        .py_4()
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(entry.path.join(".")),
                )
                .child(muted(entry.type_description.clone(), cx)),
        )
        .child(field("Value", format_value(entry.value.as_ref()), cx))
        .when_some(view.staged.get(&entry.path), |this, staged| {
            this.child(field("Staged", setting_text(staged.as_ref()), cx))
        })
        .when_some(queued, |this, queued| {
            this.child(field("Queued", setting_text(queued.as_ref()), cx))
        })
        .when_some(entry.default.clone(), |this, default| {
            this.child(field("Default", default, cx))
        })
        .when_some(entry.example.clone(), |this, example| {
            this.child(field("Example", example, cx))
        })
        .child(field("Status", status, cx))
        .when(locked.is_none(), |this| {
            this.child(editor(app, view, entry, ours, cx))
        })
        .when_some(entry.description.clone(), |this, description| {
            this.child(div().text_sm().child(plain_markdown(&description)))
        })
        .into_any_element()
}

fn editor(
    app: &NixboxApp,
    view: &OptionsView,
    entry: &OptionEntry,
    ours: bool,
    cx: &mut Context<NixboxApp>,
) -> AnyElement {
    let current = view.current(entry);
    let nullable = matches!(entry.kind, OptionKind::Nullable(_));
    let control = match entry.kind.inner() {
        OptionKind::Bool if !nullable => Switch::new("option-bool")
            .checked(matches!(current, Some(SettingValue::Bool(true))))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.stage_option(SettingValue::Bool(*checked), cx);
            }))
            .into_any_element(),
        OptionKind::Bool | OptionKind::Enum(_) => {
            let values = choices(&entry.kind);
            ButtonGroup::new("option-choice")
                .outline()
                .small()
                .children(values.iter().enumerate().map(|(index, value)| {
                    Button::new(("choice", index))
                        .label(value.to_nix())
                        .selected(current.as_ref() == Some(value))
                }))
                .on_click(cx.listener(move |this, clicked: &Vec<usize>, _, cx| {
                    if let Some(value) = clicked.first().and_then(|index| values.get(*index)) {
                        this.stage_option(value.clone(), cx);
                    }
                }))
                .into_any_element()
        }
        kind => v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&app.option_input)))
                    .child(
                        Button::new("option-stage")
                            .outline()
                            .label("Stage")
                            .on_click(cx.listener(|this, _, _, cx| this.stage_text(cx))),
                    ),
            )
            .child(muted(text_hint(kind, nullable), cx))
            .into_any_element(),
    };
    v_flex()
        .gap_2()
        .child(control)
        .when(ours || view.staged.contains_key(&entry.path), |this| {
            this.child(
                h_flex().child(
                    Button::new("option-unset")
                        .ghost()
                        .small()
                        .label(if ours { "Unset" } else { "Drop staged change" })
                        .on_click(cx.listener(|this, _, _, cx| this.unset_option(cx))),
                ),
            )
        })
        .into_any_element()
}

fn field(label: &'static str, value: String, cx: &Context<NixboxApp>) -> impl IntoElement {
    h_flex()
        .gap_3()
        .items_start()
        .child(div().w_20().flex_none().child(muted(label, cx)))
        .child(div().flex_1().min_w_0().text_sm().child(value))
}

fn text_hint(kind: &OptionKind, nullable: bool) -> &'static str {
    match kind {
        OptionKind::StrList => "Comma-separated; empty for [ ]. Press Enter or Stage.",
        OptionKind::Int | OptionKind::Float if nullable => {
            "A number; empty for null. Press Enter or Stage."
        }
        OptionKind::Int | OptionKind::Float => "A number. Press Enter or Stage.",
        _ if nullable => "Empty for null. Press Enter or Stage.",
        _ => "Used as written. Press Enter or Stage.",
    }
}

fn setting_text(value: Option<&SettingValue>) -> String {
    value.map_or_else(|| "unset".to_string(), SettingValue::to_nix)
}
