mod decimal;
mod engine;
mod keypad;
mod keys;
mod settings;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use decimal::{Decimal, Failure};
pub use engine::{Angle, Base, Binary, Constant, Engine, Input, Mode, Unary, WORD_SIZES};
pub use keypad::Keypad;
pub use settings::{base_name, mode_name, parse_base, parse_mode, restore, save};

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-calculator")));
