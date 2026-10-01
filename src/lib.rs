pub mod dsl;
pub mod editor;

pub type Buffer = String;
pub type Error = String;

pub const MAX_BUFFER_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EXPRESSION_BYTES: usize = 16 * 1024;
