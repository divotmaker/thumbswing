//! Clubs and the player's hand.

use std::fmt;
use std::str::FromStr;

/// A club with a profile in the club table.
///
/// Codes follow the Flight Relay Protocol (`DR`, `7I`, `PT`, ...).
/// [`Club::from_code`] also accepts the codes without a row of their own and
/// maps each to the nearest row (`MD` → `DR`, `2W`/`4W` → `3W`, `6W`–`9W` →
/// `5W`, any hybrid → `3H`, `1I`/`2I` → `3I`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Club {
    /// Driver.
    Driver,
    /// 3-wood.
    Wood3,
    /// 5-wood.
    Wood5,
    /// Hybrid (every hybrid uses this row).
    Hybrid3,
    /// 3-iron.
    Iron3,
    /// 4-iron.
    Iron4,
    /// 5-iron.
    Iron5,
    /// 6-iron.
    Iron6,
    /// 7-iron, the club in play until one is selected.
    #[default]
    Iron7,
    /// 8-iron.
    Iron8,
    /// 9-iron.
    Iron9,
    /// Pitching wedge.
    PitchingWedge,
    /// Gap wedge.
    GapWedge,
    /// Sand wedge.
    SandWedge,
    /// Lob wedge.
    LobWedge,
    /// Putter.
    Putter,
}

impl Club {
    /// Every club, driver through putter.
    pub const ALL: [Club; 16] = [
        Club::Driver,
        Club::Wood3,
        Club::Wood5,
        Club::Hybrid3,
        Club::Iron3,
        Club::Iron4,
        Club::Iron5,
        Club::Iron6,
        Club::Iron7,
        Club::Iron8,
        Club::Iron9,
        Club::PitchingWedge,
        Club::GapWedge,
        Club::SandWedge,
        Club::LobWedge,
        Club::Putter,
    ];

    /// The club's FRP code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Club::Driver => "DR",
            Club::Wood3 => "3W",
            Club::Wood5 => "5W",
            Club::Hybrid3 => "3H",
            Club::Iron3 => "3I",
            Club::Iron4 => "4I",
            Club::Iron5 => "5I",
            Club::Iron6 => "6I",
            Club::Iron7 => "7I",
            Club::Iron8 => "8I",
            Club::Iron9 => "9I",
            Club::PitchingWedge => "PW",
            Club::GapWedge => "GW",
            Club::SandWedge => "SW",
            Club::LobWedge => "LW",
            Club::Putter => "PT",
        }
    }

    /// The club for an FRP code, case-insensitive. Codes without a row of
    /// their own map to the nearest row; unknown codes give `None`.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Club> {
        let code = code.trim().to_ascii_uppercase();
        Some(match code.as_str() {
            "DR" | "MD" => Club::Driver,
            "2W" | "3W" | "4W" => Club::Wood3,
            "5W" | "6W" | "7W" | "8W" | "9W" => Club::Wood5,
            "1H" | "2H" | "3H" | "4H" | "5H" | "6H" | "7H" | "8H" | "9H" => Club::Hybrid3,
            "1I" | "2I" | "3I" => Club::Iron3,
            "4I" => Club::Iron4,
            "5I" => Club::Iron5,
            "6I" => Club::Iron6,
            "7I" => Club::Iron7,
            "8I" => Club::Iron8,
            "9I" => Club::Iron9,
            "PW" => Club::PitchingWedge,
            "GW" => Club::GapWedge,
            "SW" => Club::SandWedge,
            "LW" => Club::LobWedge,
            "PT" => Club::Putter,
            _ => return None,
        })
    }

    /// The next club toward the putter, wrapping to the driver.
    #[must_use]
    pub fn next(self) -> Club {
        let i = Club::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Club::ALL[(i + 1) % Club::ALL.len()]
    }

    /// The next club toward the driver, wrapping to the putter.
    #[must_use]
    pub fn prev(self) -> Club {
        let i = Club::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Club::ALL[(i + Club::ALL.len() - 1) % Club::ALL.len()]
    }

    /// Whether this is the putter.
    #[must_use]
    pub const fn is_putter(self) -> bool {
        matches!(self, Club::Putter)
    }
}

impl fmt::Display for Club {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// An unknown club code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown club code {0:?}")]
pub struct UnknownClub(pub String);

impl FromStr for Club {
    type Err = UnknownClub;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Club::from_code(s).ok_or_else(|| UnknownClub(s.to_owned()))
    }
}

/// The player's hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Hand {
    /// Right-handed.
    #[default]
    Right,
    /// Left-handed.
    Left,
}

impl Hand {
    /// `RH` or `LH`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Hand::Right => "RH",
            Hand::Left => "LH",
        }
    }

    /// The other hand.
    #[must_use]
    pub const fn other(self) -> Hand {
        match self {
            Hand::Right => Hand::Left,
            Hand::Left => Hand::Right,
        }
    }
}

impl fmt::Display for Hand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// An unknown hand.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown hand {0:?} (rh or lh)")]
pub struct UnknownHand(pub String);

impl FromStr for Hand {
    type Err = UnknownHand;

    /// `rh` or `lh` (also `right` or `left`), case-insensitive.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rh" | "right" => Ok(Hand::Right),
            "lh" | "left" => Ok(Hand::Left),
            _ => Err(UnknownHand(s.to_owned())),
        }
    }
}

/// A club is written as its FRP code and read from any code
/// [`Club::from_code`] accepts.
#[cfg(feature = "config")]
impl serde::Serialize for Club {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.code())
    }
}

#[cfg(feature = "config")]
impl<'de> serde::Deserialize<'de> for Club {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let code = String::deserialize(d)?;
        code.parse().map_err(serde::de::Error::custom)
    }
}

/// A hand is written as `rh` or `lh` and read as [`Hand::from_str`] does.
#[cfg(feature = "config")]
impl serde::Serialize for Hand {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Hand::Right => "rh",
            Hand::Left => "lh",
        })
    }
}

#[cfg(feature = "config")]
impl<'de> serde::Deserialize<'de> for Hand {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let hand = String::deserialize(d)?;
        hand.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for club in Club::ALL {
            assert_eq!(Club::from_code(club.code()), Some(club));
            assert_eq!(club.code().parse::<Club>(), Ok(club));
        }
    }

    #[test]
    fn codes_without_a_row_map_to_the_nearest_row() {
        assert_eq!(Club::from_code("MD"), Some(Club::Driver));
        assert_eq!(Club::from_code("4w"), Some(Club::Wood3));
        assert_eq!(Club::from_code("7W"), Some(Club::Wood5));
        assert_eq!(Club::from_code("5H"), Some(Club::Hybrid3));
        assert_eq!(Club::from_code("2I"), Some(Club::Iron3));
        assert_eq!(Club::from_code("ZZ"), None);
    }

    #[test]
    fn hands_parse_either_way() {
        assert_eq!("rh".parse::<Hand>(), Ok(Hand::Right));
        assert_eq!(" LH ".parse::<Hand>(), Ok(Hand::Left));
        assert_eq!("left".parse::<Hand>(), Ok(Hand::Left));
        assert!("x".parse::<Hand>().is_err());
    }

    #[test]
    fn next_and_prev_wrap() {
        assert_eq!(Club::Putter.next(), Club::Driver);
        assert_eq!(Club::Driver.prev(), Club::Putter);
        assert_eq!(Club::Iron7.next().prev(), Club::Iron7);
    }
}
