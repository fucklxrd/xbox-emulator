#![allow(non_snake_case, clippy::missing_safety_doc)]

#[cfg(windows)]
pub mod sys;
#[cfg(windows)]
pub mod msvc;
#[cfg(windows)]
pub mod bena;
#[cfg(windows)]
pub mod printf;
pub mod pe;
pub mod exports;

#[cfg(test)]
mod tests;
