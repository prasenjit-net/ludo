//! The timer, presence and room lifetime rules, driven with a made-up clock.

use super::*;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

const TIMING: Timing = Timing {
    turn: Duration::from_secs(10),
    away_turn: Duration::from_secs(15),
    idle_timeouts: 3,
    room_grace: Duration::from_secs(900),
    lobby_grace: Duration::from_secs(60),
};

/// The reconnect outcome in the `Welcome`, which comes first.
fn outcome(rx: &mut UnboundedReceiver<ServerMsg>) -> Option<Resumed> {
    match rx.try_recv() {
        Ok(ServerMsg::Welcome { resume, .. }) => resume,
        other => panic!("expected a welcome, got {other:?}"),
    }
}

struct World {
    hub: Hub,
    now: Instant,
}

impl World {
    fn new() -> Self {
        World {
            hub: Hub::new(TIMING),
            now: Instant::now(),
        }
    }

    fn connect(&self, conn: u64, client: &str) -> UnboundedReceiver<ServerMsg> {
        self.rejoin(conn, client, None)
    }

    /// Connect, claiming to be in `room`.
    fn rejoin(&self, conn: u64, client: &str, room: Option<u32>) -> UnboundedReceiver<ServerMsg> {
        let (tx, rx) = unbounded_channel();
        let hello = Hello {
            client_id: client.into(),
            name: client.into(),
            color: Player::Red,
            room,
        };
        self.hub.hello(conn, tx, hello, self.now);
        rx
    }

    /// Let time pass, ticking as the server does.
    fn run(&mut self, secs: u64) {
        for _ in 0..secs * 4 {
            self.now += Duration::from_millis(250);
            self.hub.tick(self.now);
        }
    }

    fn room<T>(&self, f: impl FnOnce(&Room) -> T) -> Option<T> {
        self.hub.lock().rooms.values().next().map(f)
    }

    /// A started game between "a" (connection 1) and "b" (connection 2).
    fn duel(&self) {
        self.connect(1, "a");
        self.connect(2, "b");
        self.hub.create_room(1, "duel", 2, self.now);
        self.hub.join_room(2, 1, self.now);
    }

    /// The seat of the person on this client id.
    fn seat(&self, client: &str) -> usize {
        self.room(|r| r.members.iter().position(|m| m.client_id == client))
            .flatten()
            .expect("seated")
    }

    fn turn(&self) -> usize {
        self.room(|r| r.game.as_ref().unwrap().turn).unwrap()
    }

    fn wait_left(&self) -> Duration {
        self.room(|r| r.deadline.unwrap() - self.now).unwrap()
    }
}

#[test]
fn an_offline_player_keeps_their_seat_however_long_they_are_gone() {
    let mut w = World::new();
    w.duel();
    let b = w.seat("b");
    w.hub.disconnect(2, w.now);
    w.run(200);
    assert!(w.room(|r| !r.game.as_ref().unwrap().left[b]).unwrap());

    let mut rx = w.rejoin(3, "b", Some(1));
    assert_eq!(outcome(&mut rx), Some(Resumed::Game));
    assert!(matches!(
        rx.try_recv(),
        Ok(ServerMsg::State { you, .. }) if you == b
    ));
}

#[test]
fn a_connected_player_who_never_acts_goes_idle_and_is_not_removed() {
    let mut w = World::new();
    w.duel();
    w.run(200);
    let (idle, left) = w
        .room(|r| {
            let idle = r.members.iter().all(|m| m.idle && m.conn.is_some());
            (idle, r.game.as_ref().unwrap().left.clone())
        })
        .unwrap();
    assert!(idle);
    assert_eq!(left, [false, false]);
}

#[test]
fn acting_ends_idleness() {
    let w = World::new();
    w.duel();
    let turn = w.turn();
    let conn = {
        let mut inner = w.hub.lock();
        let room = inner.rooms.values_mut().next().unwrap();
        room.members[turn].idle = true;
        room.members[turn].timeouts = 3;
        room.members[turn].conn.unwrap()
    };
    w.hub.act(conn, None, w.now, |g| g.roll(3));
    assert!(
        w.room(|r| !r.members[turn].idle && r.members[turn].timeouts == 0)
            .unwrap()
    );
}

#[test]
fn dropping_on_your_turn_gives_the_longer_wait_and_returning_restores_it() {
    let w = World::new();
    w.duel();
    assert_eq!(w.wait_left(), TIMING.turn);
    let (client, conn) = w
        .room(|r| {
            let m = &r.members[r.game.as_ref().unwrap().turn];
            (m.client_id.clone(), m.conn.unwrap())
        })
        .unwrap();
    w.hub.disconnect(conn, w.now);
    assert_eq!(w.wait_left(), TIMING.away_turn);
    w.connect(3, &client);
    assert_eq!(w.wait_left(), TIMING.turn);
}

/// Drop and restore the current player every `half_secs` until the server plays.
/// Returns how long that took.
fn flap_until_played(half_millis: u64) -> Duration {
    let mut w = World::new();
    w.duel();
    let (client, mut conn) = w
        .room(|r| {
            let m = &r.members[r.game.as_ref().unwrap().turn];
            (m.client_id.clone(), m.conn.unwrap())
        })
        .unwrap();
    // Any step changes the game; whose turn it is next depends on the dice.
    let before = w.room(|r| r.game.clone()).unwrap();
    let start = w.now;
    let mut next = 10;
    let mut online = true;
    while w.room(|r| r.game.clone()).unwrap() == before && w.now - start < TIMING.room_grace {
        if online {
            w.hub.disconnect(conn, w.now);
        } else {
            next += 1;
            conn = next;
            w.connect(conn, &client);
        }
        online = !online;
        for _ in 0..half_millis / 250 {
            w.now += Duration::from_millis(250);
            w.hub.tick(w.now);
        }
    }
    w.now - start
}

#[test]
fn a_flapping_connection_cannot_hold_up_the_turn() {
    let limit = TIMING.away_turn + TIMING.turn + MIN_RETURN;
    for half_millis in [500, 2_000, 4_000, 9_000] {
        let took = flap_until_played(half_millis);
        // Plus one flap, since the loop only looks between them.
        assert!(
            took <= limit + Duration::from_millis(half_millis),
            "flapping every {half_millis} ms held the turn for {took:?}"
        );
    }
}

#[test]
fn a_game_with_nobody_connected_pauses_then_resumes() {
    let mut w = World::new();
    w.duel();
    w.hub.disconnect(1, w.now);
    w.hub.disconnect(2, w.now);
    let tokens = w.room(|r| r.game.clone()).unwrap();
    w.run(600);
    assert_eq!(w.room(|r| r.game.clone()).unwrap(), tokens);
    assert!(w.room(|r| r.deadline.is_none()).unwrap());

    w.connect(3, "a");
    assert!(
        w.room(|r| r.deadline.is_some() && r.empty_since.is_none())
            .unwrap()
    );
}

#[test]
fn an_abandoned_game_expires() {
    let mut w = World::new();
    w.duel();
    w.hub.disconnect(1, w.now);
    w.hub.disconnect(2, w.now);
    w.run(899);
    assert!(w.room(|_| ()).is_some());
    w.run(2);
    assert!(w.room(|_| ()).is_none());
}

#[test]
fn a_game_everyone_left_is_removed_at_once() {
    let w = World::new();
    w.connect(1, "a");
    w.hub.create_room(1, "solo", 4, w.now);
    w.hub.start(1, w.now);
    w.hub.leave(1, w.now);
    assert!(w.room(|_| ()).is_none());
}

#[test]
fn a_waiting_room_keeps_a_dropped_host_for_a_while() {
    let mut w = World::new();
    w.connect(1, "a");
    w.connect(2, "b");
    w.hub.create_room(1, "lobby", 4, w.now);
    w.hub.join_room(2, 1, w.now);
    w.hub.disconnect(1, w.now);
    w.run(30);
    assert!(w.room(|r| r.members.len() == 2 && r.host == "a").unwrap());

    w.connect(3, "a");
    w.run(120);
    assert!(w.room(|r| r.members.len() == 2 && r.host == "a").unwrap());

    w.hub.disconnect(3, w.now);
    w.run(61);
    assert!(w.room(|r| r.members.len() == 1 && r.host == "b").unwrap());
}

#[test]
fn an_empty_waiting_room_is_removed_after_the_grace_period() {
    let mut w = World::new();
    w.connect(1, "a");
    w.hub.create_room(1, "alone", 4, w.now);
    w.hub.disconnect(1, w.now);
    w.run(59);
    assert!(w.room(|_| ()).is_some());
    w.run(2);
    assert!(w.room(|_| ()).is_none());
}

#[test]
fn a_late_request_is_rejected() {
    let w = World::new();
    let mut a = w.connect(1, "a");
    w.connect(2, "b");
    w.hub.create_room(1, "duel", 2, w.now);
    w.hub.join_room(2, 1, w.now);
    let (conn, before) = w
        .room(|r| {
            let m = &r.members[r.game.as_ref().unwrap().turn];
            (m.conn.unwrap(), r.rev)
        })
        .unwrap();
    while a.try_recv().is_ok() {}
    w.hub.act(conn, Some(before), w.now, |g| g.roll(3));
    assert_eq!(w.room(|r| r.rev).unwrap(), before + 1);
    let after = w.room(|r| r.game.clone()).unwrap();

    // The same tap again, against what is no longer on screen.
    w.hub.act(conn, Some(before), w.now, |g| g.roll(6));
    assert_eq!(w.room(|r| r.game.clone()).unwrap(), after);
    if conn == 1 {
        let stale = std::iter::from_fn(|| a.try_recv().ok()).any(|m| {
            matches!(
                m,
                ServerMsg::Error {
                    code: ErrorCode::Stale,
                    ..
                }
            )
        });
        assert!(stale);
    }
}

#[test]
fn a_returner_is_told_what_became_of_the_game() {
    // Finished: "b" wins by "a" leaving while "b" is away.
    let w = World::new();
    w.duel();
    w.hub.disconnect(2, w.now);
    w.hub.leave(1, w.now);
    let finished = Resumed::Finished { winner: "b".into() };
    assert_eq!(outcome(&mut w.rejoin(3, "b", Some(1))), Some(finished));
    assert_eq!(
        outcome(&mut w.rejoin(4, "a", Some(1))),
        Some(Resumed::Finished { winner: "b".into() })
    );
    assert_eq!(
        outcome(&mut w.rejoin(5, "stranger", Some(1))),
        Some(Resumed::NotFound)
    );

    let mut w = World::new();
    w.duel();
    w.hub.disconnect(1, w.now);
    w.hub.disconnect(2, w.now);
    w.run(901);
    assert_eq!(
        outcome(&mut w.rejoin(3, "a", Some(1))),
        Some(Resumed::Expired)
    );
    assert_eq!(
        outcome(&mut w.rejoin(4, "a", Some(7))),
        Some(Resumed::NotFound)
    );
    assert_eq!(outcome(&mut w.connect(5, "c")), None);
}

#[test]
fn leaving_one_tab_is_reported_to_the_next() {
    let w = World::new();
    w.connect(1, "a");
    w.connect(2, "b");
    w.connect(3, "c");
    w.hub.create_room(1, "trio", 3, w.now);
    w.hub.join_room(2, 1, w.now);
    w.hub.join_room(3, 1, w.now);
    w.hub.leave(2, w.now);
    assert_eq!(outcome(&mut w.rejoin(4, "b", Some(1))), Some(Resumed::Left));
}
