pub trait Indicator: std::fmt::Debug {
    fn enter_scroll(&mut self) {}
    fn exit_scroll(&mut self) {}
}

#[derive(Debug, Default)]
pub struct NoopIndicator;

impl Indicator for NoopIndicator {}
