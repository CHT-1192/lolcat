// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! Colour engine state: the output mode (truecolor vs 256-colour) and the
//! one-shot mode detection/selection used by every paint path.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ColorMode {
    Truecolor,
    Pal256,
}

pub(crate) struct Engine {
    pub(crate) os: f64,
    pub(crate) paint_init: bool,
    pub(crate) mode: ColorMode,
}

impl Engine {
    pub(crate) fn new() -> Engine {
        Engine {
            os: 0.0,
            paint_init: false,
            mode: ColorMode::Pal256,
        }
    }
}

/// Truecolor if `--truecolor` or `COLORTERM ∈ {truecolor, 24bit}`.
pub(crate) fn set_mode(eng: &mut Engine, truecolor: bool) {
    let detected = match std::env::var("COLORTERM").as_deref() {
        Ok("truecolor") | Ok("24bit") => ColorMode::Truecolor,
        _ => ColorMode::Pal256,
    };
    eng.mode = if truecolor {
        ColorMode::Truecolor
    } else {
        detected
    };
    if std::env::var_os("LOLCAT_DEBUG").is_some() {
        let num = |m: ColorMode| {
            if m == ColorMode::Truecolor {
                16777215
            } else {
                256
            }
        };
        eprintln!(
            "DEBUG: Paint.mode = {} (detected: {})",
            num(eng.mode),
            num(detected)
        );
    }
    eng.paint_init = true;
}
