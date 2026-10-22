//! The tracker: one picture of the other aircraft, made from everything that is heard about
//! them, from any source.
//!
//! Each source names an aircraft in its own way: MAVLink by system ID, Remote ID by the
//! transmitter's address, ADS-B by the ICAO address. The tracker keeps one *track* for each
//! aircraft. A new name that appears where a track already is, from a source that track has
//! not been heard on, is the same aircraft: it joins the track, so that the map shows one
//! marker. Tracks go stale when they are quiet, and go away when they have been quiet a long
//! time. Nothing here knows about radios or bytes: observations in, tracks out.

use std::collections::BTreeSet;
use std::fmt;

/// A point on the earth. Altitude is metres above sea level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub lat: f64,
    pub lon: f64,
    pub alt: f32,
}

const EARTH_RADIUS: f64 = 6_371_000.0;

impl Position {
    pub fn new(lat: f64, lon: f64, alt: f32) -> Position {
        Position { lat, lon, alt }
    }

    /// How far north and east `other` is from this point (m), on a flat earth. Good for the
    /// few kilometres that radios reach.
    pub fn offset_to(&self, other: &Position) -> (f32, f32) {
        let north = (other.lat - self.lat).to_radians() * EARTH_RADIUS;
        let east = (other.lon - self.lon).to_radians() * EARTH_RADIUS * self.lat.to_radians().cos();
        (north as f32, east as f32)
    }

    /// The distance along the ground to `other` (m).
    pub fn distance_to(&self, other: &Position) -> f32 {
        let (n, e) = self.offset_to(other);
        n.hypot(e)
    }

    /// The point `north` and `east` of this one (m), `up` metres higher.
    pub fn moved(&self, north: f32, east: f32, up: f32) -> Position {
        Position {
            lat: self.lat + (north as f64 / EARTH_RADIUS).to_degrees(),
            lon: self.lon + (east as f64 / (EARTH_RADIUS * self.lat.to_radians().cos())).to_degrees(),
            alt: self.alt + up,
        }
    }
}

/// Where a report came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    Mavlink,
    RemoteId,
    Adsb,
}

impl Source {
    /// The short name that the map puts on a label.
    pub fn label(self) -> &'static str {
        match self {
            Source::Mavlink => "MAV",
            Source::RemoteId => "RID",
            Source::Adsb => "ADSB",
        }
    }
}

/// How one source names one aircraft.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identity {
    pub source: Source,
    /// The system ID, the transmitter address or the ICAO address, as text.
    pub key: String,
}

impl Identity {
    pub fn new(source: Source, key: impl Into<String>) -> Identity {
        Identity { source, key: key.into() }
    }
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.source.label(), self.key)
    }
}

/// One report about one aircraft.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub identity: Identity,
    /// When the station heard it (s on the station's clock).
    pub time: f64,
    pub position: Position,
    /// North, east and down speed (m/s), if the report has it.
    pub velocity: Option<[f32; 3]>,
    /// Track over the ground (degrees), if the report has it.
    pub heading: Option<f32>,
    /// What the aircraft calls itself: a call sign, a Remote ID, a name.
    pub name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Heard a moment ago.
    Live,
    /// Quiet for a while: the position is where the aircraft was.
    Stale,
}

/// One aircraft, as the tracker knows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: u32,
    pub name: String,
    /// The last reported position.
    pub position: Position,
    pub velocity: Option<[f32; 3]>,
    pub heading: Option<f32>,
    /// The sources that have reported it, in a fixed order.
    pub sources: Vec<Source>,
    pub identities: Vec<Identity>,
    pub first_seen: f64,
    pub last_seen: f64,
    pub freshness: Freshness,
}

impl Track {
    /// The sources, as a label: `MAV RID`.
    pub fn source_label(&self) -> String {
        self.sources.iter().map(|s| s.label()).collect::<Vec<_>>().join(" ")
    }

    /// Where the aircraft probably is `now`, if it kept its velocity (at most 10 s ahead).
    pub fn estimate(&self, now: f64) -> Position {
        let dt = (now - self.last_seen).clamp(0.0, 10.0) as f32;
        match self.velocity {
            Some([n, e, d]) => self.position.moved(n * dt, e * dt, -d * dt),
            None => self.position,
        }
    }
}

/// How the tracker decides.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    /// A track that has been quiet this long (s) is stale.
    pub stale_after: f64,
    /// A track that has been quiet this long (s) goes away.
    pub drop_after: f64,
    /// Two reports are one aircraft when they are this near on the ground (m)...
    pub join_distance: f32,
    /// ...and this near in height (m)...
    pub join_height: f32,
    /// ...and their velocities differ by less than this (m/s).
    pub join_speed: f32,
    /// If a second track is nearly as near as the nearest (within this many metres), the
    /// choice is a guess, so the tracker does not join.
    pub join_margin: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            stale_after: 5.0,
            drop_after: 60.0,
            join_distance: 60.0,
            join_height: 40.0,
            join_speed: 8.0,
            join_margin: 10.0,
        }
    }
}

/// The picture.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    rules: Rules,
    tracks: Vec<Track>,
    next_id: u32,
}

impl Tracker {
    pub fn new() -> Tracker {
        Tracker::default()
    }

    pub fn with_rules(rules: Rules) -> Tracker {
        Tracker { rules, ..Tracker::default() }
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    /// Take in one report.
    pub fn observe(&mut self, obs: Observation) {
        let index =
            self.tracks.iter().position(|t| t.identities.contains(&obs.identity)).or_else(|| self.joinable(&obs));
        let index = index.unwrap_or_else(|| {
            self.next_id += 1;
            self.tracks.push(Track {
                id: self.next_id,
                name: obs.identity.to_string(),
                position: obs.position,
                velocity: obs.velocity,
                heading: obs.heading,
                sources: vec![],
                identities: vec![],
                first_seen: obs.time,
                last_seen: obs.time,
                freshness: Freshness::Live,
            });
            self.tracks.len() - 1
        });
        let t = &mut self.tracks[index];
        if !t.identities.contains(&obs.identity) {
            t.identities.push(obs.identity.clone());
            t.identities.sort();
        }
        let sources: BTreeSet<Source> = t.identities.iter().map(|i| i.source).collect();
        t.sources = sources.into_iter().collect();
        // An older report (heard late, or repeated) does not move the aircraft back.
        if obs.time >= t.last_seen {
            t.position = obs.position;
            t.velocity = obs.velocity.or(t.velocity);
            t.heading = obs.heading.or(t.heading);
            t.last_seen = obs.time;
            t.freshness = Freshness::Live;
        }
        if let Some(n) = obs.name {
            t.name = n;
        }
    }

    /// The track that `obs` is the same aircraft as, if there is exactly one such track.
    fn joinable(&self, obs: &Observation) -> Option<usize> {
        let r = &self.rules;
        let mut near: Vec<(usize, f32)> = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.identities.iter().any(|i| i.source == obs.identity.source))
            .filter_map(|(i, t)| {
                let there = t.estimate(obs.time);
                let d = there.distance_to(&obs.position);
                // An altitude that is not known (NaN) cannot rule a track out.
                let high = (there.alt - obs.position.alt).abs();
                let high = if high.is_nan() { 0.0 } else { high };
                let speed = match (t.velocity, obs.velocity) {
                    (Some(a), Some(b)) => ((a[0] - b[0]).hypot(a[1] - b[1])).min(f32::MAX),
                    _ => 0.0,
                };
                (d <= r.join_distance && high <= r.join_height && speed <= r.join_speed).then_some((i, d))
            })
            .collect();
        near.sort_by(|a, b| a.1.total_cmp(&b.1));
        match near.as_slice() {
            [(i, _)] => Some(*i),
            [(i, a), (_, b), ..] if b - a > r.join_margin => Some(*i),
            _ => None,
        }
    }

    /// The tracks as they are `now`, nearest-heard first: stale ones marked, and the ones that
    /// have been quiet too long left out.
    pub fn tracks(&self, now: f64) -> Vec<Track> {
        let mut out: Vec<Track> = self
            .tracks
            .iter()
            .filter(|t| now - t.last_seen < self.rules.drop_after)
            .map(|t| {
                let mut t = t.clone();
                t.freshness =
                    if now - t.last_seen < self.rules.stale_after { Freshness::Live } else { Freshness::Stale };
                t
            })
            .collect();
        out.sort_by(|a, b| b.last_seen.total_cmp(&a.last_seen).then(a.id.cmp(&b.id)));
        out
    }

    /// Forget the tracks that have been quiet too long. (`tracks` already leaves them out.)
    pub fn prune(&mut self, now: f64) {
        let drop_after = self.rules.drop_after;
        self.tracks.retain(|t| now - t.last_seen < drop_after);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: Position = Position { lat: 37.87, lon: -122.27, alt: 50.0 };

    fn heard(source: Source, key: &str, time: f64, at: Position) -> Observation {
        Observation {
            identity: Identity::new(source, key),
            time,
            position: at,
            velocity: None,
            heading: None,
            name: None,
        }
    }

    #[test]
    fn a_new_name_makes_a_new_track() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::Mavlink, "3", 1.0, HOME.moved(500.0, 0.0, 0.0)));
        assert_eq!(t.tracks(1.0).len(), 2);
    }

    #[test]
    fn the_same_name_updates_its_track() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::Mavlink, "2", 2.0, HOME.moved(20.0, 0.0, 0.0)));
        let tracks = t.tracks(2.0);
        assert_eq!(tracks.len(), 1);
        assert!(tracks[0].position.distance_to(&HOME.moved(20.0, 0.0, 0.0)) < 0.1);
        assert_eq!((tracks[0].first_seen, tracks[0].last_seen), (1.0, 2.0));
    }

    #[test]
    fn two_sources_at_the_same_place_are_one_aircraft() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::RemoteId, "aa:bb", 1.2, HOME.moved(8.0, 3.0, 2.0)));
        let tracks = t.tracks(1.5);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].sources, vec![Source::Mavlink, Source::RemoteId]);
        assert_eq!(tracks[0].source_label(), "MAV RID");
        assert_eq!(tracks[0].identities.len(), 2);
    }

    #[test]
    fn once_joined_each_name_updates_the_same_track() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::RemoteId, "aa:bb", 1.1, HOME));
        t.observe(heard(Source::RemoteId, "aa:bb", 2.1, HOME.moved(30.0, 0.0, 0.0)));
        t.observe(heard(Source::Mavlink, "2", 2.2, HOME.moved(32.0, 0.0, 0.0)));
        assert_eq!(t.tracks(2.5).len(), 1);
    }

    #[test]
    fn two_sources_far_apart_are_two_aircraft() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::RemoteId, "aa:bb", 1.0, HOME.moved(400.0, 0.0, 0.0)));
        assert_eq!(t.tracks(1.0).len(), 2);
    }

    #[test]
    fn a_different_height_is_a_different_aircraft() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::RemoteId, "aa:bb", 1.0, HOME.moved(5.0, 0.0, 120.0)));
        assert_eq!(t.tracks(1.0).len(), 2);
    }

    #[test]
    fn an_unknown_height_does_not_stop_two_sources_joining() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::RemoteId, "aa:bb", 1.0, Position { alt: f32::NAN, ..HOME.moved(5.0, 0.0, 0.0) }));
        assert_eq!(t.tracks(1.0).len(), 1);
    }

    #[test]
    fn a_different_velocity_is_a_different_aircraft() {
        let mut t = Tracker::new();
        let mut a = heard(Source::Mavlink, "2", 1.0, HOME);
        a.velocity = Some([20.0, 0.0, 0.0]);
        let mut b = heard(Source::RemoteId, "aa:bb", 1.0, HOME);
        b.velocity = Some([-20.0, 0.0, 0.0]);
        t.observe(a);
        t.observe(b);
        assert_eq!(t.tracks(1.0).len(), 2, "two planes that cross, one each way");
    }

    #[test]
    fn two_sources_of_one_kind_do_not_join() {
        // Two MAVLink systems that happen to be near are two planes, as in a game of tag.
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::Mavlink, "3", 1.0, HOME.moved(1.5, 0.0, 0.0)));
        assert_eq!(t.tracks(1.0).len(), 2);
    }

    #[test]
    fn a_guess_between_two_near_tracks_is_not_made() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::Mavlink, "3", 1.0, HOME.moved(3.0, 0.0, 0.0)));
        t.observe(heard(Source::RemoteId, "aa:bb", 1.0, HOME.moved(1.5, 0.0, 0.0)));
        assert_eq!(t.tracks(1.0).len(), 3, "the Remote ID is equally near both, so it stands alone");
    }

    #[test]
    fn the_nearest_of_two_tracks_wins_when_it_is_clearly_nearest() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 1.0, HOME));
        t.observe(heard(Source::Mavlink, "3", 1.0, HOME.moved(40.0, 0.0, 0.0)));
        t.observe(heard(Source::RemoteId, "aa:bb", 1.0, HOME.moved(2.0, 0.0, 0.0)));
        let tracks = t.tracks(1.0);
        assert_eq!(tracks.len(), 2);
        let joined = tracks.iter().find(|t| t.sources.len() == 2).unwrap();
        assert!(joined.identities.contains(&Identity::new(Source::Mavlink, "2")));
    }

    #[test]
    fn a_moving_track_is_found_where_it_has_gone() {
        let mut t = Tracker::new();
        let mut a = heard(Source::Mavlink, "2", 0.0, HOME);
        a.velocity = Some([20.0, 0.0, 0.0]);
        t.observe(a);
        // 2 s later the plane is 40 m north; the Remote ID gives that, and 40 m is within the gate only if the track is moved on.
        let mut b = heard(Source::RemoteId, "aa:bb", 2.0, HOME.moved(90.0, 0.0, 0.0));
        b.velocity = Some([20.0, 0.0, 0.0]);
        t.observe(b);
        assert_eq!(t.tracks(2.0).len(), 1, "90 m is 50 m from where the plane has got to");
    }

    #[test]
    fn a_quiet_track_goes_stale_and_then_goes_away() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 10.0, HOME));
        assert_eq!(t.tracks(12.0)[0].freshness, Freshness::Live);
        assert_eq!(t.tracks(16.0)[0].freshness, Freshness::Stale);
        assert_eq!(t.tracks(69.0).len(), 1);
        assert_eq!(t.tracks(71.0).len(), 0);
        t.prune(71.0);
        t.observe(heard(Source::Mavlink, "2", 80.0, HOME));
        assert_eq!(t.tracks(80.0)[0].freshness, Freshness::Live);
    }

    #[test]
    fn a_late_report_does_not_move_the_aircraft_back() {
        let mut t = Tracker::new();
        t.observe(heard(Source::Mavlink, "2", 5.0, HOME.moved(100.0, 0.0, 0.0)));
        t.observe(heard(Source::Mavlink, "2", 4.0, HOME));
        assert!(t.tracks(5.0)[0].position.distance_to(&HOME.moved(100.0, 0.0, 0.0)) < 0.1);
    }

    #[test]
    fn a_name_that_the_aircraft_gives_is_kept() {
        let mut t = Tracker::new();
        t.observe(heard(Source::RemoteId, "aa:bb", 1.0, HOME));
        assert_eq!(t.tracks(1.0)[0].name, "RID aa:bb");
        let mut o = heard(Source::RemoteId, "aa:bb", 2.0, HOME);
        o.name = Some("T1S-0042".into());
        t.observe(o);
        t.observe(heard(Source::RemoteId, "aa:bb", 3.0, HOME));
        assert_eq!(t.tracks(3.0)[0].name, "T1S-0042");
    }

    #[test]
    fn positions_move_by_metres() {
        let p = HOME.moved(300.0, -400.0, 10.0);
        let (n, e) = HOME.offset_to(&p);
        assert!((n - 300.0).abs() < 0.5 && (e + 400.0).abs() < 0.5, "{n} {e}");
        assert!((HOME.distance_to(&p) - 500.0).abs() < 1.0);
        assert_eq!(p.alt, 60.0);
    }

    #[test]
    fn an_estimate_runs_on_at_the_velocity_for_a_few_seconds() {
        let mut t = Tracker::new();
        let mut o = heard(Source::Mavlink, "2", 0.0, HOME);
        o.velocity = Some([10.0, 0.0, -1.0]);
        t.observe(o);
        let tr = &t.tracks(0.0)[0];
        assert!(tr.estimate(2.0).distance_to(&HOME.moved(20.0, 0.0, 0.0)) < 0.5);
        assert!((tr.estimate(2.0).alt - 52.0).abs() < 1e-3);
        assert!(tr.estimate(100.0).distance_to(&HOME.moved(100.0, 0.0, 0.0)) < 0.5, "it stops at 10 s");
    }
}
