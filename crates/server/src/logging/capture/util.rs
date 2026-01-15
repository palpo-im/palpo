use std::sync::{Arc, Mutex};

use super::super::{Level, fmt};
use super::{Closure, Data};
use crate::AppResult;

pub fn fmt_html<S>(out: Arc<Mutex<S>>) -> Box<Closure>
where
    S: std::fmt::Write + Send + 'static,
{
    fmt(fmt::html, out)
}

pub fn fmt_markdown<S>(out: Arc<Mutex<S>>) -> Box<Closure>
where
    S: std::fmt::Write + Send + 'static,
{
    fmt(fmt::markdown, out)
}

pub fn fmt<F, S>(fun: F, out: Arc<Mutex<S>>) -> Box<Closure>
where
    F: Fn(&mut S, &Level, &str, &str) -> AppResult<()> + Send + Sync + Copy + 'static,
    S: std::fmt::Write + Send + 'static,
{
    Box::new(move |data| call(fun, &mut *out.lock().expect("locked"), &data))
}

fn call<F, S>(fun: F, out: &mut S, data: &Data<'_>)
where
    F: Fn(&mut S, &Level, &str, &str) -> AppResult<()>,
    S: std::fmt::Write,
{
    fun(out, &data.level(), data.span_name(), data.message()).expect("log line appended");
}
