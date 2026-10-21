/// Diffuser intensity: 1 (low), 2 (medium) or 3 (high).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Level(u8);

impl Level {
    /// `Some` for 1..=3, `None` for anything else.
    pub fn new(value: u8) -> Option<Level> {
        (1..=3).contains(&value).then_some(Level(value))
    }

    pub fn get(&self) -> u8 {
        self.0
    }
}
