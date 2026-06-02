use crate::key::KeyIntent;
use crate::predict::{BasePredictor, Overlay, PredictorPlugin};
use crate::screen::Screen;

#[derive(Debug, Clone)]
pub struct ExampleApplicationPredictor {
    base: BasePredictor,
}

impl ExampleApplicationPredictor {
    pub fn new(enabled: bool) -> Self {
        Self {
            base: BasePredictor::new(enabled),
        }
    }
}

impl PredictorPlugin for ExampleApplicationPredictor {
    fn name(&self) -> &'static str {
        "example-application"
    }

    fn overlay(&self) -> &Overlay {
        self.base.overlay()
    }

    fn on_key(&mut self, intent: KeyIntent, screen: &Screen) {
        self.base.on_key(intent, screen);
    }

    fn reconcile(&mut self, screen: &Screen) {
        self.base.reconcile(screen);
    }

    fn clear(&mut self) {
        self.base.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::KeyIntent;
    use crate::screen::{Screen, Size};

    #[test]
    fn delegates_printable_prediction_to_base() {
        let screen = Screen::new(Size { cols: 10, rows: 2 });
        let mut predictor = ExampleApplicationPredictor::new(true);

        predictor.on_key(KeyIntent::Printable('x'), &screen);

        assert_eq!(predictor.name(), "example-application");
        assert_eq!(predictor.overlay().cells.len(), 1);
        assert_eq!(predictor.overlay().cells[0].cell.ch, 'x');
    }
}
