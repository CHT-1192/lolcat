// Copyright (c) 2016, moe@busyloop.net
// ... (BSD 3-Clause, see LICENSE)
//
//! SIGINT plumbing and the terminal-restore guard: an RAII `ResetGuard`
//! that restores the terminal at clean exits, plus a Ctrl-C handler that
//! drains piped upstream output on the first interrupt and force-restores
//! the screen on a second one.

use std::io::{self, Write};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Terminal restore guard — RAII equivalent of Ruby's `ensure` block.
pub(crate) struct ResetGuard {
    pub(crate) tty: bool,
}

impl Drop for ResetGuard {
    fn drop(&mut self) {
        if self.tty {
            let mut out = io::stdout();
            let _ = out.write_all(END_RESET);
            let _ = out.flush();
        }
    }
}

/// Normal end-of-run reset: colour off, cursor visible, bracketed paste
/// off. Deliberately no `ESC[?1049l` — at a clean EOF the upstream program
/// already sent its own leave-alternate-screen sequence, and emitting one
/// here unconditionally made Terminal.app clear the viewport when a huge
/// single-line scrollback was present.
const END_RESET: &[u8] = b"\x1b[m\x1b[?25h\x1b[?1;5;2004l";

/// Terminal reset emitted when lolcat itself has to force-restore the
/// screen on Ctrl-C. Unlike END_RESET it also leaves the alternate screen
/// buffer, because on a hard exit the upstream program's own `ESC[?1049l`
/// may have been lost.
const TERM_RESET: &[u8] = b"\x1b[?1049l\x1b[m\x1b[?25h\x1b[?1;5;2004l";

/// SIGINT handling: when the upstream side is a pipe, the first Ctrl-C only
/// flags the interrupt so lolcat keeps draining stdin — the producer got
/// the same signal and its exit sequences (`ESC[?1049l`, `ESC[?25h`, …)
/// must still be forwarded, otherwise full-screen TUIs would be left stuck
/// in the alternate screen buffer. A second Ctrl-C forces an immediate exit.
static INTERRUPTS: AtomicU8 = AtomicU8::new(0);
static DRAIN_ON_INT: AtomicBool = AtomicBool::new(false);

pub(crate) fn install_ctrlc_handler(tty: bool, drain: bool) {
    DRAIN_ON_INT.store(drain, Ordering::Relaxed);
    let _ = ctrlc::set_handler(move || {
        let n = INTERRUPTS.fetch_add(1, Ordering::Relaxed) + 1;
        let draining = DRAIN_ON_INT.load(Ordering::Relaxed);
        if !draining {
            // Interactive use (stdin is a terminal): nothing is in the
            // alternate screen, so reset WITHOUT `ESC[?1049l` — emitting it
            // here made Terminal.app clear the viewport. A trailing newline
            // stops zsh drawing its reversed end-of-output `%` marker.
            if tty {
                let mut out = io::stdout();
                let _ = out.write_all(END_RESET);
                let _ = out.write_all(b"\n");
                let _ = out.flush();
            }
            process::exit(130);
        }
        // Draining a pipe: the first Ctrl-C only flags the interrupt so the
        // upstream program's exit sequences (ESC[?1049l, ...) still reach
        // the terminal before EOF. A second one forces exit; there we may
        // genuinely be inside the upstream's alternate screen, so TERM_RESET
        // (which includes leaving it) is used.
        if n >= 2 {
            if tty {
                let mut out = io::stdout();
                let _ = out.write_all(TERM_RESET);
                let _ = out.write_all(b"\n");
                let _ = out.flush();
            }
            process::exit(130);
        }
    });
}
