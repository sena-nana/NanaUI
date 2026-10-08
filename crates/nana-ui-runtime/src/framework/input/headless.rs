//! A source with no window: the input adapter for headless sessions, tests
//! and offscreen harnesses. It stamps events on its own clock, routes them
//! through the same path a window's take, and keeps what the host would have
//! been told in [`HeadlessHostServices`].

use std::time::Duration;

use nana_ui_input::{
    CanonicalInputEvent, CommittedText, CompositionInput, DeviceId, EndpointGeneration,
    HeadlessHostServices, InputPayload, InputSequencer, InputSourceId, KeyInput, PointerInput,
    PointerPhase,
};

use super::{InputBindError, InputRouteError, InputRouteOutcome};
use crate::{AppContext, DocumentId, TextShaper};

/// One headless input source bound to one document.
#[derive(Debug, Clone)]
pub struct HeadlessInput {
    sequencer: InputSequencer,
    services: HeadlessHostServices,
    now: Duration,
}

impl HeadlessInput {
    /// The source a plain [`Self::bind`] uses.
    pub const SOURCE: InputSourceId = InputSourceId(1);

    /// Bind [`Self::SOURCE`] to `document`, taking over whatever binding the
    /// source had. Taking over revokes the previous source's captures,
    /// presses and hover state before the new generation is installed.
    pub fn bind(context: &mut AppContext, document: DocumentId) -> Self {
        let previous = context.input_binding(Self::SOURCE);
        let generation = previous.map_or(EndpointGeneration(1), |(generation, _)| {
            // A generation must never repeat: wrapping would make an
            // event from a retired source indistinguishable from a live
            // one. Keep this in lockstep with `InputSequencer::advance`.
            EndpointGeneration(
                generation
                    .0
                    .checked_add(1)
                    .expect("input endpoint generation exhausted"),
            )
        });
        if previous.is_some() {
            context
                .unbind_input_source(Self::SOURCE, Duration::ZERO)
                .expect("headless source cleanup must succeed");
        }
        context
            .bind_input_source(Self::SOURCE, generation, document)
            .expect("a newer generation always binds");
        Self::bound(Self::SOURCE, generation)
    }

    /// Bind `source` to `document` at `generation`.
    pub fn bind_source(
        context: &mut AppContext,
        source: InputSourceId,
        generation: EndpointGeneration,
        document: DocumentId,
    ) -> Result<Self, InputBindError> {
        context.bind_input_source(source, generation, document)?;
        Ok(Self::bound(source, generation))
    }

    fn bound(source: InputSourceId, generation: EndpointGeneration) -> Self {
        Self {
            sequencer: InputSequencer::new(source, generation),
            services: HeadlessHostServices::new(),
            now: Duration::ZERO,
        }
    }

    pub fn generation(&self) -> EndpointGeneration {
        self.sequencer.generation()
    }

    /// The time the next event is stamped with.
    pub fn now(&self) -> Duration {
        self.now
    }

    /// Move the clock on. Time only moves when told to, so a scripted
    /// session stays reproducible.
    pub fn advance(&mut self, by: Duration) {
        self.now = self.now.saturating_add(by);
    }

    /// Set the clock. Earlier than the last event is rejected when routed.
    pub fn set_now(&mut self, now: Duration) {
        self.now = now;
    }

    pub fn services(&self) -> &HeadlessHostServices {
        &self.services
    }

    pub fn services_mut(&mut self) -> &mut HeadlessHostServices {
        &mut self.services
    }

    /// Stamp `payload` as this source's next event without routing it.
    pub fn stamp(&mut self, payload: InputPayload) -> CanonicalInputEvent {
        CanonicalInputEvent {
            metadata: self.sequencer.stamp(DeviceId(0), self.now),
            payload,
        }
    }

    pub fn route(
        &mut self,
        context: &mut AppContext,
        payload: InputPayload,
    ) -> Result<InputRouteOutcome, InputRouteError> {
        self.route_shaped(context, payload, None)
    }

    pub fn route_shaped(
        &mut self,
        context: &mut AppContext,
        payload: InputPayload,
        shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputRouteOutcome, InputRouteError> {
        let event = self.stamp(payload);
        context.route_input(&event, &mut self.services, shaper)
    }

    /// A primary mouse sample at `(x, y)`.
    pub fn pointer(
        &mut self,
        context: &mut AppContext,
        phase: PointerPhase,
        x: f32,
        y: f32,
    ) -> Result<InputRouteOutcome, InputRouteError> {
        self.route(
            context,
            InputPayload::Pointer(PointerInput::mouse(phase, x, y)),
        )
    }

    /// A key press and the text it types, delivered as a keyboard delivers
    /// them: the key, then the text naming it.
    pub fn press(
        &mut self,
        context: &mut AppContext,
        key: KeyInput,
        text: Option<&str>,
        mut shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputRouteOutcome, InputRouteError> {
        let key_event = self.stamp(InputPayload::Key(key));
        let sequence = key_event.metadata.sequence;
        let outcome = context.route_input(
            &key_event,
            &mut self.services,
            super::reborrow_text_shaper(&mut shaper),
        )?;
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return Ok(outcome);
        };
        let text_outcome = self.route_shaped(
            context,
            InputPayload::Text(CommittedText {
                text: text.to_owned(),
                key: Some(sequence),
            }),
            shaper,
        )?;
        Ok(InputRouteOutcome {
            handled: outcome.handled || text_outcome.handled,
            prevent_default: outcome.prevent_default || text_outcome.prevent_default,
            pointer_hit: None,
            invalidated_work: outcome.invalidated_work || text_outcome.invalidated_work,
            pointer_id: None,
        })
    }

    /// Text committed with no key press.
    pub fn text(
        &mut self,
        context: &mut AppContext,
        text: &str,
    ) -> Result<InputRouteOutcome, InputRouteError> {
        self.route(context, InputPayload::Text(CommittedText::new(text)))
    }

    pub fn composition(
        &mut self,
        context: &mut AppContext,
        composition: CompositionInput,
    ) -> Result<InputRouteOutcome, InputRouteError> {
        self.route(context, InputPayload::Composition(composition))
    }
}
