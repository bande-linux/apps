use crate::engine::{Angle, Base, Engine, Mode, WORD_SIZES};

const MODES: [(Mode, &str); 3] = [
    (Mode::Basic, "basic"),
    (Mode::Scientific, "scientific"),
    (Mode::Programmer, "programmer"),
];
const BASES: [(Base, &str); 4] = [
    (Base::Hexadecimal, "hexadecimal"),
    (Base::Decimal, "decimal"),
    (Base::Octal, "octal"),
    (Base::Binary, "binary"),
];

pub fn mode_name(mode: Mode) -> &'static str {
    MODES
        .iter()
        .find(|(known, _)| *known == mode)
        .map_or("basic", |(_, name)| name)
}

pub fn base_name(base: Base) -> &'static str {
    BASES
        .iter()
        .find(|(known, _)| *known == base)
        .map_or("decimal", |(_, name)| name)
}

pub fn parse_mode(name: &str) -> Option<Mode> {
    MODES
        .iter()
        .find(|(_, known)| *known == name)
        .map(|(mode, _)| *mode)
}

pub fn parse_base(name: &str) -> Option<Base> {
    BASES
        .iter()
        .find(|(_, known)| *known == name)
        .map(|(base, _)| *base)
}

pub fn save(engine: &Engine, tape: bool) -> String {
    let angle = match engine.angle {
        Angle::Degrees => "degrees",
        Angle::Radians => "radians",
    };
    format!(
        "mode {}\nrpn {}\nangle {angle}\nbase {}\nbits {}\ngrouping {}\ntape {tape}\n",
        mode_name(engine.mode),
        engine.rpn,
        base_name(engine.base),
        engine.bits,
        engine.grouping,
    )
}

pub fn restore(engine: &mut Engine, text: &str) -> bool {
    let mut tape = false;
    for (key, value) in text.lines().filter_map(|line| line.split_once(' ')) {
        match key {
            "mode" => {
                if let Some(mode) = parse_mode(value) {
                    engine.set_mode(mode);
                }
            }
            "rpn" => engine.set_rpn(value == "true"),
            "angle" => {
                engine.angle = match value {
                    "radians" => Angle::Radians,
                    _ => Angle::Degrees,
                }
            }
            "base" => {
                if let Some(base) = parse_base(value) {
                    engine.set_base(base);
                }
            }
            "bits" => {
                if let Some(bits) = value.parse().ok().filter(|bits| WORD_SIZES.contains(bits)) {
                    engine.set_bits(bits);
                }
            }
            "grouping" => engine.grouping = value != "false",
            "tape" => tape = value == "true",
            _ => {}
        }
    }
    tape
}
