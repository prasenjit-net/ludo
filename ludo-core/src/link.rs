//! When the client should connect, give up and retry. No I/O: it is told what
//! happened and the time (milliseconds), and answers with what to do. `random`
//! arguments are uniform in `0..1`.

/// An attempt that has not opened by now is abandoned.
pub const CONNECT_TIMEOUT_MS: f64 = 8_000.0;
/// An open connection this quiet is dead. Must exceed the server's ping interval.
pub const SILENCE_MS: f64 = 40_000.0;
/// When the tab is looked at again, this much quiet is enough to reconnect.
pub const PROBE_SILENCE_MS: f64 = 20_000.0;
/// Waits before successive retries; the last one repeats for ever.
const RETRY_MS: [f64; 6] = [500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 10_000.0];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    /// Throw away any existing socket and open a new one.
    Connect,
    /// Close the socket; a retry has been scheduled.
    Drop,
}

#[derive(Clone, Copy, Debug, Default)]
enum State {
    #[default]
    Stopped,
    Connecting {
        since: f64,
    },
    Open {
        heard: f64,
    },
    Waiting {
        until: f64,
    },
}

#[derive(Default, Debug)]
pub struct Link {
    state: State,
    /// Failures since the server was last heard from.
    failures: usize,
}

impl Link {
    pub const fn new() -> Self {
        Link {
            state: State::Stopped,
            failures: 0,
        }
    }

    pub fn started(&self) -> bool {
        !matches!(self.state, State::Stopped)
    }

    /// Connect now, whatever was going on.
    pub fn connect(&mut self, now: f64) -> Command {
        self.state = State::Connecting { since: now };
        Command::Connect
    }

    pub fn opened(&mut self, now: f64) {
        self.state = State::Open { heard: now };
    }

    /// Something arrived from the server. Only this resets the retry delay, so
    /// a server that accepts and then drops us is still backed off from.
    pub fn received(&mut self, now: f64) {
        if let State::Open { heard } = &mut self.state {
            *heard = now;
            self.failures = 0;
        }
    }

    /// The socket closed or could not be made.
    pub fn lost(&mut self, now: f64, random: f64) {
        if matches!(self.state, State::Stopped | State::Waiting { .. }) {
            return;
        }
        let base = RETRY_MS[self.failures.min(RETRY_MS.len() - 1)];
        self.failures += 1;
        // Spread by 30% either way so clients do not all return at once.
        self.state = State::Waiting {
            until: now + base * (0.7 + 0.6 * random),
        };
    }

    /// Called regularly.
    pub fn tick(&mut self, now: f64, random: f64) -> Option<Command> {
        let expired = match self.state {
            State::Stopped => false,
            State::Connecting { since } => now - since >= CONNECT_TIMEOUT_MS,
            State::Open { heard } => now - heard >= SILENCE_MS,
            State::Waiting { until } => return (now >= until).then(|| self.connect(now)),
        };
        expired.then(|| {
            self.lost(now, random);
            Command::Drop
        })
    }

    /// The network came back, the page was restored, or the user asked: an open
    /// socket is probably stale, so start again at once.
    pub fn restart(&mut self, now: f64) -> Option<Command> {
        if !self.started() {
            return None;
        }
        self.failures = 0;
        Some(self.connect(now))
    }

    /// The tab is being looked at again.
    pub fn visible(&mut self, now: f64) -> Option<Command> {
        match self.state {
            State::Waiting { .. } => self.restart(now),
            State::Open { heard } if now - heard >= PROBE_SILENCE_MS => self.restart(now),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How long until the link asks to connect again, with no jitter.
    fn retry_after(link: &mut Link, now: &mut f64) -> f64 {
        let from = *now;
        while link.tick(*now, 0.5) != Some(Command::Connect) {
            *now += 100.0;
        }
        *now - from
    }

    #[test]
    fn retries_back_off_then_repeat() {
        let mut link = Link::new();
        let mut now = 0.0;
        link.connect(now);
        for expected in [
            500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 10_000.0, 10_000.0,
        ] {
            link.lost(now, 0.5);
            assert_eq!(retry_after(&mut link, &mut now), expected);
        }
    }

    #[test]
    fn only_hearing_from_the_server_resets_the_delay() {
        let mut link = Link::new();
        let mut now = 0.0;
        link.connect(now);
        link.lost(now, 0.5);
        retry_after(&mut link, &mut now);
        link.opened(now);
        link.lost(now, 0.5);
        assert_eq!(retry_after(&mut link, &mut now), 1_000.0);
        link.opened(now);
        link.received(now);
        link.lost(now, 0.5);
        assert_eq!(retry_after(&mut link, &mut now), 500.0);
    }

    #[test]
    fn delays_are_jittered() {
        for (random, expected) in [(0.0, 350.0), (1.0, 650.0)] {
            let mut link = Link::new();
            link.connect(0.0);
            link.lost(0.0, random);
            assert_eq!(link.tick(expected - 1.0, 0.5), None);
            assert_eq!(link.tick(expected + 1.0, 0.5), Some(Command::Connect));
        }
    }

    #[test]
    fn a_hanging_attempt_is_abandoned() {
        let mut link = Link::new();
        link.connect(0.0);
        assert_eq!(link.tick(CONNECT_TIMEOUT_MS - 1.0, 0.5), None);
        assert_eq!(link.tick(CONNECT_TIMEOUT_MS, 0.5), Some(Command::Drop));
    }

    #[test]
    fn a_silent_connection_is_dropped() {
        let mut link = Link::new();
        link.connect(0.0);
        link.opened(0.0);
        link.received(10_000.0);
        assert_eq!(link.tick(10_000.0 + SILENCE_MS - 1.0, 0.5), None);
        assert_eq!(link.tick(10_000.0 + SILENCE_MS, 0.5), Some(Command::Drop));
    }

    #[test]
    fn becoming_visible_checks_the_connection() {
        let mut link = Link::new();
        assert_eq!(link.visible(0.0), None);
        assert_eq!(link.restart(0.0), None);
        link.connect(0.0);
        link.opened(0.0);
        assert_eq!(link.visible(1_000.0), None);
        assert_eq!(link.visible(PROBE_SILENCE_MS), Some(Command::Connect));
        link.lost(PROBE_SILENCE_MS, 0.5);
        assert_eq!(link.visible(PROBE_SILENCE_MS + 1.0), Some(Command::Connect));
    }
}
