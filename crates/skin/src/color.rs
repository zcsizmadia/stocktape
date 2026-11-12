//! Colors.

use std::fmt;
use std::str::FromStr;

/// A straight-alpha RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha; 255 is opaque.
    pub a: u8,
}

impl Color {
    /// An opaque color.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

impl FromStr for Color {
    type Err = String;

    /// Parses `#RGB`, `#RRGGBB` or `#RRGGBBAA`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || format!("invalid color {s:?}; expected #RRGGBB or #RRGGBBAA");
        let hex = s.trim().strip_prefix('#').ok_or_else(bad)?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        match hex.len() {
            3 => {
                let nibble = |i: usize| {
                    u8::from_str_radix(&hex[i..=i], 16)
                        .map(|v| v * 17)
                        .map_err(|_| bad())
                };
                Ok(Self::rgb(nibble(0)?, nibble(1)?, nibble(2)?))
            }
            6 => Ok(Self::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Ok(Self {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: byte(6)?,
            }),
            _ => Err(bad()),
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 255 {
            write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            write!(
                f,
                "#{:02X}{:02X}{:02X}{:02X}",
                self.r, self.g, self.b, self.a
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parses_forms() {
        assert_eq!("#0E1A2F".parse(), Ok(Color::rgb(0x0E, 0x1A, 0x2F)));
        assert_eq!("#fff".parse(), Ok(Color::rgb(255, 255, 255)));
        assert_eq!(
            "#FFFFFF14".parse(),
            Ok(Color {
                r: 255,
                g: 255,
                b: 255,
                a: 0x14
            })
        );
    }

    #[test]
    fn rejects_bad_input() {
        for s in ["0E1A2F", "#12", "#12345", "#GGGGGG", "#1234567", "", "#ÿÿÿ"] {
            assert!(s.parse::<Color>().is_err(), "{s}");
        }
    }

    proptest! {
        #[test]
        fn display_round_trips(r: u8, g: u8, b: u8, a: u8) {
            let c = Color { r, g, b, a };
            prop_assert_eq!(c.to_string().parse::<Color>(), Ok(c));
        }
    }
}
