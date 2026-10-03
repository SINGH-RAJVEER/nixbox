//! Everything installed: packages nixbox manages, flake outputs, and packages
//! declared by hand that can be moved into a managed file.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Disableable as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
	AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
	StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};

use super::{empty, muted, page_header, row, section};
use crate::app::NixboxApp;
use crate::model::{InstalledRows, target_of};

pub fn render(app: &mut NixboxApp, cx: &mut Context<NixboxApp>) -> AnyElement {
	let rows = InstalledRows::new(&app.session.engine, &app.installed_input.read(cx).value());
	let migratable = app
		.session
		.engine
		.external_packages
		.iter()
		.any(|package| package.migratable)
		|| app
			.session
			.engine
			.flakes
			.iter()
			.any(|flake| flake.migratable());
	let header = page_header(
		"Installed",
		"Packages nixbox manages, flake outputs, and packages declared in your own config.",
		Some(
			Button::new("migrate-all")
				.outline()
				.label("Migrate all")
				.tooltip("Move every eligible package and flake output into nixbox's managed files")
				.disabled(!migratable)
				.on_click(cx.listener(|this, _, window, cx| this.migrate_all(window, cx)))
				.into_any_element(),
		),
		cx,
	);
	let filter = div().px_6().pb_2().child(
		Input::new(&app.installed_input)
			.prefix(IconName::Search)
			.cleanable(true),
	);

	let body = if rows.is_empty() {
		empty("Nothing installed matches.", cx).into_any_element()
	} else {
		v_flex()
			.id("installed")
			.size_full()
			.overflow_y_scroll()
			.pb_4()
			.child(section("Managed by nixbox", rows.managed.len(), cx))
			.children(
				rows.managed
					.into_iter()
					.enumerate()
					.map(|(index, package)| {
						let name = package.name.clone();
						let scope = package.scope;
						let options_name = name.clone();
						row(cx)
							.child(name_cell(&package.name))
							.child(Tag::secondary().xsmall().child(scope.tag()))
							.child(
								Button::new(("options-managed", index))
									.small()
									.ghost()
									.icon(IconName::Settings)
									.label("Options")
									.on_click(cx.listener(move |this, _, window, cx| {
										this.open_options(options_name.clone(), scope, window, cx);
									})),
							)
							.child(
								Button::new(("remove-managed", index))
									.small()
									.ghost()
									.icon(IconName::Delete)
									.label("Remove")
									.on_click(cx.listener(move |this, _, window, cx| {
										this.remove_managed(name.clone(), scope, window, cx);
									})),
							)
					}),
			)
			.child(section("Flakes", rows.flakes.len(), cx))
			.children(rows.flakes.into_iter().enumerate().map(|(index, flake)| {
				let origin = flake.declared_in.clone().map_or_else(
					|| "nixbox flake module".to_string(),
					|list| format!("declared in {list}"),
				);
				let repo = flake.repo.clone().unwrap_or_else(|| flake.input.clone());
				let removable = flake.removable;
				row(cx)
					.child(
						v_flex()
							.flex_1()
							.min_w_0()
							.child(div().font_weight(FontWeight::MEDIUM).child(flake.name()))
							.child(muted(format!("{repo}, {origin}"), cx)),
					)
					.child(Tag::secondary().xsmall().child(flake.scope.tag()))
					.when(flake.migratable(), |row| {
						let flake = flake.clone();
						row.child(
							Button::new(("migrate-flake", index))
								.small()
								.ghost()
								.icon(IconName::ArrowRight)
								.label("Migrate")
								.on_click(cx.listener(move |this, _, window, cx| {
									this.migrate_flake(flake.clone(), window, cx);
								})),
						)
					})
					.child(
						Button::new(("remove-flake", index))
							.small()
							.ghost()
							.icon(IconName::Delete)
							.label("Remove")
							.disabled(!removable)
							.when(!removable, |button| {
								button
									.tooltip("Shares a line with other entries; remove it by hand")
							})
							.on_click(cx.listener(move |this, _, window, cx| {
								this.remove_flake(flake.clone(), window, cx);
							})),
					)
			}))
			.child(section("Declared in your config", rows.external.len(), cx))
			.children(
				rows.external
					.into_iter()
					.enumerate()
					.map(|(index, package)| {
						let migratable = package.migratable;
						let options_name = package.name.clone();
						let scope = target_of(package.scope);
						row(cx)
							.child(
								v_flex()
									.flex_1()
									.min_w_0()
									.child(
										div()
											.font_weight(FontWeight::MEDIUM)
											.child(package.name.clone()),
									)
									.child(muted(
										format!(
											"{}, line {}",
											package.source_attr,
											package.line + 1
										),
										cx,
									)),
							)
							.child(
								Tag::secondary()
									.xsmall()
									.child(target_of(package.scope).tag()),
							)
							.child(
								Button::new(("options-external", index))
									.small()
									.ghost()
									.icon(IconName::Settings)
									.label("Options")
									.on_click(cx.listener(move |this, _, window, cx| {
										this.open_options(options_name.clone(), scope, window, cx);
									})),
							)
							.child(
								Button::new(("migrate", index))
									.small()
									.ghost()
									.icon(IconName::ArrowRight)
									.label("Migrate")
									.disabled(!migratable)
									.on_click(cx.listener(move |this, _, window, cx| {
										this.migrate(package.clone(), window, cx);
									})),
							)
					}),
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

fn name_cell(name: &str) -> impl IntoElement {
	h_flex().flex_1().min_w_0().child(
		div()
			.font_weight(FontWeight::MEDIUM)
			.child(name.to_string()),
	)
}
