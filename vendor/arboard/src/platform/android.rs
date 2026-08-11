//! NanaUI Android clipboard stub for iced_winit.
//!
//! Upstream `arboard` has no Android backend; iced still links it on native
//! targets. This stub compiles and reports `ClipboardNotSupported` so NanaUI /
//! Iced can cross-compile to `aarch64-linux-android`. Real IME/clipboard will
//! go through `nana-ui-platform` later.

use std::{
	borrow::Cow,
	path::{Path, PathBuf},
};

use crate::common::Error;
#[cfg(feature = "image-data")]
use crate::common::ImageData;

pub(crate) struct Clipboard;

impl Clipboard {
	pub(crate) fn new() -> Result<Clipboard, Error> {
		Err(Error::ClipboardNotSupported)
	}
}

pub(crate) struct Get<'clipboard> {
	_clipboard: &'clipboard Clipboard,
}

impl<'clipboard> Get<'clipboard> {
	pub(crate) fn new(clipboard: &'clipboard mut Clipboard) -> Self {
		Self { _clipboard: clipboard }
	}

	pub(crate) fn text(self) -> Result<String, Error> {
		Err(Error::ClipboardNotSupported)
	}

	pub(crate) fn html(self) -> Result<String, Error> {
		Err(Error::ClipboardNotSupported)
	}

	pub(crate) fn file_list(self) -> Result<Vec<PathBuf>, Error> {
		Err(Error::ClipboardNotSupported)
	}

	#[cfg(feature = "image-data")]
	pub(crate) fn image(self) -> Result<ImageData<'static>, Error> {
		Err(Error::ClipboardNotSupported)
	}
}

pub(crate) struct Set<'clipboard> {
	_clipboard: &'clipboard mut Clipboard,
}

impl<'clipboard> Set<'clipboard> {
	pub(crate) fn new(clipboard: &'clipboard mut Clipboard) -> Self {
		Self { _clipboard: clipboard }
	}

	pub(crate) fn text(self, _data: Cow<'_, str>) -> Result<(), Error> {
		Err(Error::ClipboardNotSupported)
	}

	pub(crate) fn html(self, _html: Cow<'_, str>, _alt_html: Option<Cow<'_, str>>) -> Result<(), Error> {
		Err(Error::ClipboardNotSupported)
	}

	pub(crate) fn file_list(self, _file_list: &[impl AsRef<Path>]) -> Result<(), Error> {
		Err(Error::ClipboardNotSupported)
	}

	#[cfg(feature = "image-data")]
	pub(crate) fn image(self, _data: ImageData) -> Result<(), Error> {
		Err(Error::ClipboardNotSupported)
	}
}

pub(crate) struct Clear<'clipboard> {
	_clipboard: &'clipboard mut Clipboard,
}

impl<'clipboard> Clear<'clipboard> {
	pub(crate) fn new(clipboard: &'clipboard mut Clipboard) -> Self {
		Self { _clipboard: clipboard }
	}

	pub(crate) fn clear(self) -> Result<(), Error> {
		Err(Error::ClipboardNotSupported)
	}
}
