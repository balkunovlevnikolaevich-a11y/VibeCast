//! Public integration type; paid character implementations live in the private repository.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Action { Walk, Stand, LookUp }
