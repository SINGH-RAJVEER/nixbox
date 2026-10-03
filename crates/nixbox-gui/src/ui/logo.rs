//! Version control logos. The brand logos keep their colors, so they are
//! drawn as images; the fallback is a theme-tinted Lucide icon.

use std::sync::{Arc, LazyLock};

use gpui_kit::component::Icon;
use gpui_kit::{AnyElement, Image, ImageFormat, IntoElement, Pixels, Styled as _, img};
use nixbox_core::vcs::Backend;

static GIT: LazyLock<Arc<Image>> = LazyLock::new(|| svg(include_bytes!("../../assets/git.svg")));
static JJ: LazyLock<Arc<Image>> = LazyLock::new(|| svg(include_bytes!("../../assets/jj.svg")));

/// Lucide's git-branch, for a configuration that is not under version control.
const UNTRACKED: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/></svg>"#;

fn svg(bytes: &[u8]) -> Arc<Image> {
	Arc::new(Image::from_bytes(ImageFormat::Svg, bytes.to_vec()))
}

/// The logo of `backend`, or a generic branch icon when there is none.
pub fn vcs_logo(backend: Option<Backend>, size: Pixels) -> AnyElement {
	match backend {
		Some(Backend::Git) => img(GIT.clone())
			.size(size)
			.flex_shrink_0()
			.into_any_element(),
		Some(Backend::Jj) => img(JJ.clone())
			.size(size)
			.flex_shrink_0()
			.into_any_element(),
		None => Icon::default()
			.data(UNTRACKED)
			.size(size)
			.into_any_element(),
	}
}

pub fn backend_name(backend: Backend) -> &'static str {
	match backend {
		Backend::Git => "Git",
		Backend::Jj => "Jujutsu",
	}
}
