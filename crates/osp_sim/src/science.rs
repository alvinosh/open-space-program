//! Where science is done. An experiment ([`crate::data::ExperimentDef`]) works in some situations
//! over some bodies, and collects one whole set in each place: its subject, the experiment, the body
//! and the situation ([`subject`]). Over a body the situations are, by height: landed or splashed
//! down, flying low and flying high in its air, and in space low and high above it
//! ([`crate::body::Body::flying_high`], [`crate::body::Body::space_high`]).

use serde::{Deserialize, Serialize};

use crate::body::Body;

/// Where a vessel is, as experiments see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Situation {
    /// On the ground.
    Landed,
    /// On a sea.
    Splashed,
    /// In the air, below where it thins ([`Body::flying_high`]).
    FlyingLow,
    FlyingHigh,
    /// Out of the air, below [`Body::space_high`].
    SpaceLow,
    SpaceHigh,
}

impl Situation {
    pub const ALL: [Situation; 6] = [
        Situation::Landed,
        Situation::Splashed,
        Situation::FlyingLow,
        Situation::FlyingHigh,
        Situation::SpaceLow,
        Situation::SpaceHigh,
    ];

    /// As records and subjects write it.
    pub fn id(self) -> &'static str {
        match self {
            Situation::Landed => "landed",
            Situation::Splashed => "splashed",
            Situation::FlyingLow => "flying_low",
            Situation::FlyingHigh => "flying_high",
            Situation::SpaceLow => "space_low",
            Situation::SpaceHigh => "space_high",
        }
    }

    /// Over `body`, as the player reads it: "flying high over Earth".
    pub fn over(self, body: &str) -> String {
        match self {
            Situation::Landed => format!("landed on {body}"),
            Situation::Splashed => format!("splashed down on {body}"),
            Situation::FlyingLow => format!("flying low over {body}"),
            Situation::FlyingHigh => format!("flying high over {body}"),
            Situation::SpaceLow => format!("in space near {body}"),
            Situation::SpaceHigh => format!("in space high over {body}"),
        }
    }

    /// Over `body`, `altitude` m above its sea level, on the ground or a sea or neither.
    pub fn of(body: &Body, altitude: f64, landed: bool, afloat: bool) -> Self {
        if landed || afloat {
            return if afloat {
                Situation::Splashed
            } else {
                Situation::Landed
            };
        }
        if body.has_atmosphere_at(altitude) {
            if altitude < body.flying_high() {
                Situation::FlyingLow
            } else {
                Situation::FlyingHigh
            }
        } else if altitude < body.space_high() {
            Situation::SpaceLow
        } else {
            Situation::SpaceHigh
        }
    }
}

/// Where a part is, for the experiments on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// The body it is over, by name.
    pub body: String,
    pub situation: Situation,
    /// In sunlight.
    pub sunlit: bool,
}

impl Place {
    /// As the player reads it: "flying high over Earth".
    pub fn describe(&self) -> String {
        self.situation.over(&self.body)
    }
}

/// The subject of `experiment` at `place`: what one whole set is of, and what the science it is
/// worth goes by. `<experiment>@<body>.<situation>`.
pub fn subject(experiment: &str, place: &Place) -> String {
    format!("{experiment}@{}.{}", place.body, place.situation.id())
}

/// A subject's experiment, body and situation, if it is one.
pub fn parse_subject(subject: &str) -> Option<(&str, &str, Situation)> {
    let (experiment, rest) = subject.split_once('@')?;
    let (body, situation) = rest.rsplit_once('.')?;
    let situation = Situation::ALL.into_iter().find(|s| s.id() == situation)?;
    Some((experiment, body, situation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subject_reads_back() {
        let place = Place {
            body: "Earth".into(),
            situation: Situation::FlyingHigh,
            sunlit: true,
        };
        let id = subject("temperature", &place);
        assert_eq!(id, "temperature@Earth.flying_high");
        assert_eq!(
            parse_subject(&id),
            Some(("temperature", "Earth", Situation::FlyingHigh))
        );
        assert_eq!(place.describe(), "flying high over Earth");
        assert_eq!(parse_subject("nothing"), None);
    }
}
