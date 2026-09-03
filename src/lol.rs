// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! Facade for the old single-module layout: re-exports the items main.rs
//! used to reach as `lol::…`, so the binary's call sites keep working
//! unchanged. The actual code now lives in the sibling modules declared in
//! [`crate`](crate) (`cat`, `options`, `engine`, `color`, `render`,
//! `stream`, `anchor`, `ansi`, `cli`, `help`, `sigint`).

pub(crate) use crate::cat::cat;
pub(crate) use crate::engine::Engine;
