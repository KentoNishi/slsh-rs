mod base;

pub mod applications;

pub use base::BasePredictor;

use crate::key::KeyIntent;
use crate::screen::{Cursor, Screen};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub enabled: bool,
    pub cells: Vec<OverlayCell>,
    pub cursor: Option<Cursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayCell {
    pub pos: Cursor,
    pub cell: crate::screen::Cell,
    pub under: crate::screen::Cell,
    pub kind: OverlayKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Printable,
    Deletion { remote_seen: bool },
}

pub trait PredictorPlugin {
    fn name(&self) -> &'static str;
    fn overlay(&self) -> &Overlay;
    fn on_key(&mut self, intent: KeyIntent, screen: &Screen);
    fn reconcile(&mut self, screen: &Screen);
    fn clear(&mut self);
}

pub fn default_predictor(enabled: bool) -> Box<dyn PredictorPlugin> {
    match std::env::var("SLSH_PREDICTOR").ok().as_deref() {
        Some("example-application") => Box::new(
            applications::example::ExampleApplicationPredictor::new(enabled),
        ),
        _ => Box::new(BasePredictor::new(enabled)),
    }
}
