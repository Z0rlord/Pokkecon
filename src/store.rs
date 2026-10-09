use crate::model::Signal;

pub trait Store {
    fn put(&mut self, s: Signal);
    fn all(&self) -> Vec<Signal>;
}

#[derive(Default)]
pub struct MemStore(Vec<Signal>);

impl Store for MemStore {
    fn put(&mut self, s: Signal) {
        self.0.push(s);
    }
    fn all(&self) -> Vec<Signal> {
        self.0.clone()
    }
}
