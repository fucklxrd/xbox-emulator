pub mod api;
pub mod client;
pub mod com;
pub mod json;
pub mod pix_client;
pub mod server;
#[cfg(test)]
mod server_tests;
#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod pix_client_tests;

#[no_mangle]
pub extern "system" fn XtfPixPCLogPerformanceResult() -> i32 { 0 }
#[cfg(test)] mod tests { use super::*; #[test] fn log_performance_result_returns_s_ok() { assert_eq!(XtfPixPCLogPerformanceResult(), 0); } }
