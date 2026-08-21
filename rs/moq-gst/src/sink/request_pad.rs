//! The `sink_%u` request pad as its own GObject, so the track it publishes can be named from a
//! pipeline description through `GstChildProxy` (`moqsink sink_0::track=camera`).

use std::sync::{LazyLock, Mutex};

use gst::glib;
use gst::prelude::*;
use gst::subclass::prelude::*;

use super::session::CAT;

#[derive(Debug, Default)]
struct Settings {
	/// The name asked for through the property. Outlives the producer, so a restarted element reserves
	/// the same name again.
	requested: Option<String>,
	/// The name the broadcast actually reserved. Present only while that producer lives, and while it is
	/// present the property is fixed.
	effective: Option<String>,
}

#[derive(Debug, Default)]
pub struct MoqSinkPadImp {
	settings: Mutex<Settings>,
}

#[glib::object_subclass]
impl ObjectSubclass for MoqSinkPadImp {
	const NAME: &'static str = "MoqSinkPad";
	type Type = MoqSinkPad;
	type ParentType = gst::Pad;
}

impl ObjectImpl for MoqSinkPadImp {
	fn properties() -> &'static [glib::ParamSpec] {
		static PROPS: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
			vec![
				// MUTABLE_PLAYING because a pad requested while the element runs is configurable right
				// there; the window closes at this pad's CAPS event, which no state-based flag can express.
				glib::ParamSpecString::builder("track")
					.nick("Track")
					.blurb(
						"Name this pad publishes as, both in the broadcast and in the catalog. Writable \
						 in any state until the CAPS event reserves the track, and read-only from then \
						 on, when it reads back the reserved name. Going back to READY releases the \
						 reservation and makes it writable again. Empty keeps the generated name \
						 (0.avc3, 0.aac, ...)",
					)
					.mutable_playing()
					.build(),
			]
		});
		PROPS.as_ref()
	}

	fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
		let mut settings = self.settings.lock().unwrap();
		// A producer keeps the name it reserved for its whole life, so a later write would read back
		// without ever reaching the broadcast or the catalog.
		if settings.effective.is_some() {
			gst::warning!(
				CAT,
				obj = self.obj(),
				"{} ignored: the track is already reserved",
				pspec.name()
			);
			return;
		}
		match pspec.name() {
			// An empty name is not a track name: it selects the generated one.
			"track" => settings.requested = value.get::<Option<String>>().unwrap().filter(|name| !name.is_empty()),
			_ => unreachable!(),
		}
	}

	fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
		let settings = self.settings.lock().unwrap();
		match pspec.name() {
			"track" => settings
				.effective
				.clone()
				.or_else(|| settings.requested.clone())
				.to_value(),
			_ => unreachable!(),
		}
	}
}

impl GstObjectImpl for MoqSinkPadImp {}
impl PadImpl for MoqSinkPadImp {}

glib::wrapper! {
	/// A `moqsink` request pad: one track of the broadcast.
	pub struct MoqSinkPad(ObjectSubclass<MoqSinkPadImp>) @extends gst::Pad, gst::Object;
}

impl MoqSinkPad {
	/// The name configured for this pad, read at the CAPS event to reserve the track.
	pub(super) fn requested_track(&self) -> Option<String> {
		self.imp().settings.lock().unwrap().requested.clone()
	}

	/// Record the name the broadcast reserved, fixing the property for the producer's lifetime. The
	/// caller must not hold the element state lock: `notify` runs handlers that read properties.
	pub(super) fn commit_track(&self, track: String) {
		let mut settings = self.imp().settings.lock().unwrap();
		let before = settings.effective.clone().or_else(|| settings.requested.clone());
		settings.effective = Some(track);
		let changed = before != settings.effective;
		drop(settings);
		if changed {
			self.notify("track");
		}
	}

	/// Drop the reserved name once its producer is finalized, so the pad is configurable again on the
	/// next run. The requested name stays: that run reserves the same one.
	pub(super) fn release_track(&self) {
		let mut settings = self.imp().settings.lock().unwrap();
		let before = settings.effective.take();
		let changed = before.is_some() && before != settings.requested;
		drop(settings);
		if changed {
			self.notify("track");
		}
	}
}
