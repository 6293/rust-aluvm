// AluRE: AluVM runtime environment.
// This is rust implementation of AluVM (arithmetic logic unit virtual machine).
//
// Designed & written in 2021 by
//     Dr. Maxim Orlovsky <orlovsky@pandoracore.com>
//
// This software is licensed under the terms of MIT License.
// You should have received a copy of the MIT License along with this software.
// If not, see <https://opensource.org/licenses/MIT>.

use core::fmt::{self, Display, Formatter, LowerExp, LowerHex, Octal, UpperExp, UpperHex, Write};
use core::hash::{Hash, Hasher};
use core::ops::{
    Deref, Index, IndexMut, Range, RangeFrom, RangeFull, RangeInclusive, RangeTo, RangeToInclusive,
};
use core::str::FromStr;

use amplify_num::{u1024, u256, u512};
use rustc_apfloat::ieee;

use crate::reg::reg::RegisterSet;

/// Trait of different number layouts
pub trait NumberLayout: Copy {
    /// Returns how many bits are used by the layout
    #[inline]
    fn bits(self) -> u16 { self.bytes() * 8 }

    /// Returns how many bytes are used by the layout
    fn bytes(self) -> u16;

    /// Returns whether layout supports signed numbers
    fn is_signed(self) -> bool;

    /// Detects whether layout uses fixed number of bits or may be applied to the numbers with
    /// arbitrary bit size.
    #[inline]
    fn is_fixed_width(self) -> bool { true }

    /// Returns bit number which keeps (or may be used to store) sign information
    fn sign_bit(self) -> u16;

    /// Returns byte number which keeps (or may be used to store) sign information
    fn sign_byte(self) -> u16;
}

/// Layout of the value encoding.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Display)]
#[display(inner)]
pub enum Layout {
    /// Integer layout
    Integer(IntLayout),

    /// Float layouts
    Float(FloatLayout),
}

impl Layout {
    /// Returns signed integer layout
    #[inline]
    pub fn signed(bytes: u16) -> Layout { Layout::Integer(IntLayout::signed(bytes)) }

    /// Returns unsigned integer layout
    #[inline]
    pub fn unsigned(bytes: u16) -> Layout { Layout::Integer(IntLayout::unsigned(bytes)) }

    /// Constructs float layout
    #[inline]
    pub fn float(layout: FloatLayout) -> Layout { Layout::Float(layout) }

    /// Detects if the number layout is unsigned integer
    #[inline]
    pub fn is_unsigned_int(self) -> bool {
        matches!(self, Layout::Integer(IntLayout { signed: false, .. }))
    }

    /// Detects if the number layout is signed integer
    #[inline]
    pub fn is_signed_int(self) -> bool {
        matches!(self, Layout::Integer(IntLayout { signed: true, .. }))
    }

    /// Detects if the number layout is one of integer (signed or unsigned) layouts
    #[inline]
    pub fn is_integer(self) -> bool { matches!(self, Layout::Integer(_)) }

    /// Detects if the number layout is one of float layouts
    #[inline]
    pub fn is_float(self) -> bool { matches!(self, Layout::Float(_)) }

    /// Converts unsigned integer layout into signed; does nothing for float layouts
    #[inline]
    pub fn into_signed(mut self) -> Layout {
        match self {
            Layout::Integer(ref mut il) => *il = il.into_signed(),
            _ => {}
        }
        self
    }

    /// Updates integer layout (if used) to match signed/unsigned format of some other layout.
    /// Does nothing if any of the layouts are not integer layouts or `other` layout is unsigned.
    #[inline]
    pub fn using_sign(mut self, other: Layout) -> Layout {
        match (self, other) {
            (Layout::Integer(ref mut il), Layout::Integer(il2)) => *il = il2.using_sign(il2),
            _ => {}
        }
        self
    }
}

impl NumberLayout for Layout {
    #[inline]
    fn bytes(self) -> u16 {
        match self {
            Layout::Integer(il) => il.bytes(),
            Layout::Float(fl) => fl.bytes(),
        }
    }

    #[inline]
    fn is_signed(self) -> bool { matches!(self, Layout::Integer(IntLayout { signed: true, .. })) }

    #[inline]
    fn sign_bit(self) -> u16 {
        match self {
            Layout::Integer(il) => il.sign_bit(),
            Layout::Float(fl) => fl.sign_bit(),
        }
    }

    #[inline]
    fn sign_byte(self) -> u16 {
        match self {
            Layout::Integer(il) => il.sign_byte(),
            Layout::Float(fl) => fl.sign_byte(),
        }
    }
}

/// Layout of the integer value encoding.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct IntLayout {
    /// Format of the integer (signed or unsigned).
    ///
    /// Unsigned integer: exact correspondence of bits to bytes in little-endian bit format
    ///
    /// Signed integer: the most significant bit (highest bit) indicates value sign. For the
    /// negative numbers the value is modulo-divided by the maximum number.
    pub signed: bool,

    /// Number of bytes occupied by the number
    pub bytes: u16,
}

impl Display for IntLayout {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_char(if self.signed { 'i' } else { 'u' })?;
        write!(f, "{}", self.bits())
    }
}

impl IntLayout {
    /// Returns signed integer layout
    #[inline]
    pub fn signed(bytes: u16) -> IntLayout { Self { signed: true, bytes } }

    /// Returns unsigned integer layout
    #[inline]
    pub fn unsigned(bytes: u16) -> IntLayout { Self { signed: false, bytes } }

    /// Converts unsigned integer layout into signed
    #[inline]
    pub fn into_signed(mut self) -> IntLayout {
        self.signed = true;
        self
    }

    /// Updates layout (if used) to match signed/unsigned format of some other layout.
    #[inline]
    pub fn using_sign(mut self, other: IntLayout) -> IntLayout {
        self.signed = other.signed;
        self
    }
}

impl NumberLayout for IntLayout {
    #[inline]
    fn bytes(self) -> u16 { self.bytes }

    #[inline]
    fn is_signed(self) -> bool { self.signed }

    #[inline]
    fn sign_bit(self) -> u16 { self.bits() - 1 }

    #[inline]
    fn sign_byte(self) -> u16 { self.bytes() - 1 }
}

/// Layout of the float value encoding.
///
/// Defines bit dimensionality and encoding format for float types.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Display)]
pub enum FloatLayout {
    /// 16-bit bfloat16 format used in machine learning
    #[display("bfloat16")]
    BFloat16 = 2,

    /// 16-bit IEEE-754 binary16 half-precision
    #[display("ieee:binary16")]
    IeeeHalf = 3,

    /// 32-bit IEEE-754 binary32 single-precision
    #[display("ieee:binary32")]
    IeeeSingle = 4,

    /// 64-bit IEEE-754 binary64 double-precision
    #[display("ieee:binary64")]
    IeeeDouble = 5,

    /// 80-bit IEEE-754 extended precision
    #[display("x87:binary80")]
    X87DoubleExt = 6,

    /// 128-bit IEEE-754 binary128 quadruple precision
    #[display("ieee:binary128")]
    IeeeQuad = 7,

    /// 256-bit IEEE-754 binary256 octuple precision
    #[display("ieee:binary256")]
    IeeeOct = 8,

    /// 512-bit tapered floating point
    #[display("tapered:binary512")]
    FloatTapered = 9,
}

impl NumberLayout for FloatLayout {
    fn bytes(self) -> u16 {
        match self {
            FloatLayout::BFloat16 => 2,
            FloatLayout::IeeeHalf => 2,
            FloatLayout::IeeeSingle => 4,
            FloatLayout::IeeeDouble => 8,
            FloatLayout::X87DoubleExt => 10,
            FloatLayout::IeeeQuad => 16,
            FloatLayout::IeeeOct => 32,
            FloatLayout::FloatTapered => 64,
        }
    }

    #[inline]
    fn is_signed(self) -> bool { true }

    #[inline]
    fn sign_bit(self) -> u16 { self.bits() - 1 }

    #[inline]
    fn sign_byte(self) -> u16 { self.bytes() - 1 }
}

impl FloatLayout {
    /// Detects if layout is used for encoding floating-point numbers
    #[inline]
    pub fn is_float(self) -> bool { self as u8 > 1 }

    /// Detects if this layout uses variable significand/exponent size
    #[inline]
    pub fn is_tapered(self) -> bool { self == FloatLayout::FloatTapered }

    /// For float numbers returns range of bits used by significand. For integer numbers always
    /// `None`.
    #[inline]
    pub fn significand_pos(self) -> Option<Range<u16>> {
        match self {
            FloatLayout::BFloat16 => Some(0..7),
            FloatLayout::IeeeHalf => Some(0..10),
            FloatLayout::IeeeSingle => Some(0..23),
            FloatLayout::IeeeDouble => Some(0..52),
            FloatLayout::X87DoubleExt => Some(0..64),
            FloatLayout::IeeeQuad => Some(0..112),
            FloatLayout::IeeeOct => Some(0..236),
            FloatLayout::FloatTapered => None,
        }
    }

    /// For float numbers returns range of bits used by exponent. For integer numbers always `None`.
    #[inline]
    pub fn exponent_pos(self) -> Option<Range<u16>> {
        match self {
            FloatLayout::BFloat16 => Some(7..15),
            FloatLayout::IeeeHalf => Some(10..15),
            FloatLayout::IeeeSingle => Some(23..31),
            FloatLayout::IeeeDouble => Some(52..63),
            FloatLayout::X87DoubleExt => Some(64..79),
            FloatLayout::IeeeQuad => Some(112..127),
            FloatLayout::IeeeOct => Some(236..255),
            FloatLayout::FloatTapered => None,
        }
    }
}

/// Representation of the value from a register, which may be `None` if the register is unset.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default, From)]
pub struct MaybeNumber(Option<Number>);

impl MaybeNumber {
    /// Creates [`MaybeNumber`] without assigning a value to it
    #[inline]
    pub fn none() -> MaybeNumber { MaybeNumber(None) }

    /// Creates [`MaybeNumber`] assigning a value to it
    #[inline]
    pub fn some(val: Number) -> MaybeNumber { MaybeNumber(Some(val)) }

    /// Transforms internal value layout returning whether this was possible without discarding any
    /// bit information
    #[inline]
    pub fn reshape(&mut self, to: Layout) -> bool {
        match self.0 {
            None => true,
            Some(ref mut val) => val.reshape(to),
        }
    }
}

impl From<Number> for MaybeNumber {
    fn from(val: Number) -> Self { MaybeNumber(Some(val)) }
}

impl From<&Number> for MaybeNumber {
    fn from(val: &Number) -> Self { MaybeNumber(Some(*val)) }
}

impl From<&Option<Number>> for MaybeNumber {
    fn from(val: &Option<Number>) -> Self { MaybeNumber(*val) }
}

impl From<Option<&Number>> for MaybeNumber {
    fn from(val: Option<&Number>) -> Self { MaybeNumber(val.copied()) }
}

impl From<MaybeNumber> for Option<Number> {
    fn from(val: MaybeNumber) -> Self { val.0 }
}

impl Deref for MaybeNumber {
    type Target = Option<Number>;

    fn deref(&self) -> &Self::Target { &self.0 }
}

impl Display for MaybeNumber {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.0 {
            None => f.write_str("~"),
            Some(ref val) => Display::fmt(val, f),
        }
    }
}

/// Type holding number of any layout
#[derive(Copy, Clone, Debug)]
pub struct Number {
    /// Internal number representation, up to the possible maximum size of any supported number
    /// layout
    bytes: [u8; 1024],

    /// Number layout used by the value
    layout: Layout,
}

impl Hash for Number {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let clean = self.to_clean();
        clean.layout.hash(state);
        state.write(&clean.bytes);
    }
}

impl Default for Number {
    fn default() -> Number {
        Number { layout: Layout::Integer(IntLayout::unsigned(1)), bytes: [0u8; 1024] }
    }
}

impl AsRef<[u8]> for Number {
    fn as_ref(&self) -> &[u8] { &self[..] }
}

impl AsMut<[u8]> for Number {
    fn as_mut(&mut self) -> &mut [u8] { &mut self[..] }
}

impl Index<u16> for Number {
    type Output = u8;

    fn index(&self, index: u16) -> &Self::Output {
        assert!(index < self.len());
        &self.bytes[index as usize]
    }
}

impl IndexMut<u16> for Number {
    fn index_mut(&mut self, index: u16) -> &mut Self::Output {
        assert!(index < self.len());
        &mut self.bytes[index as usize]
    }
}

impl Index<RangeFull> for Number {
    type Output = [u8];

    fn index(&self, _: RangeFull) -> &Self::Output { &self.bytes[..self.len() as usize] }
}

impl IndexMut<RangeFull> for Number {
    fn index_mut(&mut self, _: RangeFull) -> &mut Self::Output {
        &mut self.bytes[..self.len() as usize]
    }
}

impl Index<Range<u16>> for Number {
    type Output = [u8];

    fn index(&self, index: Range<u16>) -> &Self::Output {
        assert!(index.start < self.len() && index.end <= self.len());
        &self.bytes[index.start as usize..index.end as usize]
    }
}

impl IndexMut<Range<u16>> for Number {
    fn index_mut(&mut self, index: Range<u16>) -> &mut Self::Output {
        assert!(index.start < self.len() && index.end <= self.len());
        &mut self.bytes[index.start as usize..index.end as usize]
    }
}

impl Index<RangeInclusive<u16>> for Number {
    type Output = [u8];

    fn index(&self, index: RangeInclusive<u16>) -> &Self::Output {
        assert!(*index.start() < self.len() && *index.end() < self.len());
        &self.bytes[*index.start() as usize..*index.end() as usize]
    }
}

impl IndexMut<RangeInclusive<u16>> for Number {
    fn index_mut(&mut self, index: RangeInclusive<u16>) -> &mut Self::Output {
        &mut self.bytes[*index.start() as usize..*index.end() as usize]
    }
}

impl Index<RangeFrom<u16>> for Number {
    type Output = [u8];

    fn index(&self, index: RangeFrom<u16>) -> &Self::Output {
        assert!(index.start < self.len());
        &self.bytes[index.start as usize..self.len() as usize]
    }
}

impl IndexMut<RangeFrom<u16>> for Number {
    fn index_mut(&mut self, index: RangeFrom<u16>) -> &mut Self::Output {
        assert!(index.start < self.len());
        &mut self.bytes[index.start as usize..self.len() as usize]
    }
}

impl Index<RangeTo<u16>> for Number {
    type Output = [u8];

    fn index(&self, index: RangeTo<u16>) -> &Self::Output {
        assert!(index.end <= self.len());
        &self.bytes[..index.end as usize]
    }
}

impl IndexMut<RangeTo<u16>> for Number {
    fn index_mut(&mut self, index: RangeTo<u16>) -> &mut Self::Output {
        assert!(index.end <= self.len());
        &mut self.bytes[..index.end as usize]
    }
}

impl Index<RangeToInclusive<u16>> for Number {
    type Output = [u8];

    fn index(&self, index: RangeToInclusive<u16>) -> &Self::Output {
        assert!(index.end < self.len());
        &self.bytes[..=index.end as usize]
    }
}

impl IndexMut<RangeToInclusive<u16>> for Number {
    fn index_mut(&mut self, index: RangeToInclusive<u16>) -> &mut Self::Output {
        assert!(index.end < self.len());
        &mut self.bytes[..=index.end as usize]
    }
}

impl Number {
    /// Creates zero value with a given layout
    #[inline]
    pub fn zero(layout: Layout) -> Number { Number { layout, bytes: [0u8; 1024] } }

    #[doc(hidden)]
    /// Constructs number representation from a slice taken directly from the register
    ///
    /// Panics if the slice length does not match the registry format. Should be used only
    /// internally; for construction numbers from the values coming not from the registers use
    /// [`Number::with_layout`] method.
    #[inline]
    pub(crate) fn with_reg(slice: impl AsRef<[u8]>, reg: impl RegisterSet) -> Number {
        let mut me = Number::from_slice(slice);
        me.layout = reg.layout();
        me
    }

    /// Constructs number representation from a slice and a given layout.
    ///
    /// Fails returning `None` if the length of slice does not match the required layout byte
    /// length.
    pub fn with(slice: impl AsRef<[u8]>, layout: impl Into<Layout>) -> Option<Number> {
        let layout = layout.into();
        let slice = slice.as_ref();
        if slice.len() != layout.bytes() as usize {
            return None;
        }
        let mut me = Number::from_slice(slice);
        me.layout = layout;
        Some(me)
    }

    /// Constructs value from slice of bytes.
    ///
    /// Panics if the length of the slice is greater than 1024 bytes.
    pub fn from_slice(slice: impl AsRef<[u8]>) -> Number {
        let len = slice.as_ref().len();
        let mut bytes = [0u8; 1024];
        bytes[0..len].copy_from_slice(slice.as_ref());
        Number { layout: Layout::unsigned(len as u16), bytes }
    }

    /// Constructs value from hex string
    #[cfg(feature = "std")]
    pub fn from_hex(s: &str) -> Result<Number, amplify_num::hex::Error> {
        use amplify_num::hex::FromHex;
        let s = s.trim_start_matches("0x");
        let len = s.len() / 2;
        if len > 1024 {
            return Err(amplify_num::hex::Error::InvalidLength(1024, len));
        }
        let mut bytes = [0u8; 1024];
        let hex = Vec::<u8>::from_hex(&s)?;
        bytes[0..len].copy_from_slice(&hex);
        Ok(Number { layout: Layout::unsigned(hex.len() as u16), bytes })
    }

    /// Serializes value in hexadecimal format to a string
    #[cfg(feature = "std")]
    pub fn to_hex(self) -> String {
        let mut ret = String::with_capacity(2usize * self.len() as usize + 2);
        write!(ret, "0x").expect("writing to string");
        for ch in &self.bytes {
            write!(ret, "{:02x}", ch).expect("writing to string");
        }
        ret
    }

    /// Returns length of the used portion of the value
    #[inline]
    pub fn len(&self) -> u16 { self.layout.bytes() }

    /// Returns number layout used by the value
    #[inline]
    pub fn layout(&self) -> Layout { self.layout }

    /// Returns the number of zeros in the binary representation of `self`.
    #[inline]
    pub fn count_zeros(&self) -> u16 { self.len() - self.count_ones() }

    /// Returns the number of ones in the binary representation of `self`.
    pub fn count_ones(&self) -> u16 {
        let mut count = 0u16;
        for byte in &self[..] {
            count += byte.count_ones() as u16;
        }
        count
    }

    /// Measures minimum number of bits required to store the number. For float layouts, always
    /// matches the layout bit size.
    pub fn min_bit_len(&self) -> u16 {
        if self.layout.is_float() {
            return self.layout.bits() as u16;
        }
        if self.len() == 0 {
            return 0;
        }
        let mut data = self[..].to_vec();
        let mut sig_len = if self.layout.is_signed() {
            data[self.layout.sign_byte() as usize] &= 0x7F;
            1
        } else {
            0
        };
        let mut index = self.len() as usize - 1;
        while data[index].count_ones() > 0 && index > 0 {
            index -= 1;
        }
        index as u16 * 8 + 8 - data[index].leading_zeros() as u16 + sig_len
    }

    /// Detects if the number value positive (i.e. `>0`) or not.
    pub fn is_positive(self) -> bool {
        if self.layout.is_unsigned_int() {
            return true;
        }
        self[self.layout.sign_byte()] & 0x80 > 0
    }

    /// Detects if the value is equal to zero
    pub fn is_zero(self) -> bool {
        let mut clean = self.to_clean();
        if self.layout.is_float() {
            clean = clean.without_sign();
        }
        clean.to_u1024_bytes() == u1024::from(0u8)
    }

    /// Detects if the value is equal to the maximum possible value for the used layout. For floats,
    /// always `false`.
    pub fn is_max(self) -> bool {
        match self.layout {
            Layout::Integer(int_layout) => {
                let mut mask = u1024::from(0u8);
                for i in 0..int_layout.bytes - int_layout.is_signed() as u16 {
                    mask <<= 1;
                    mask |= 1u8;
                }
                self.to_clean() == mask.into()
            }
            _ => false,
        }
    }

    /// Ensures that all non-value bits are set to zero
    #[inline]
    pub fn clean(&mut self) { self[self.len()..].fill(0); }

    /// Returns a copy where all non-value bits are set to zero
    #[inline]
    pub fn to_clean(mut self) -> Self {
        self[self.len()..].fill(0);
        self
    }

    /// Transforms internal value layout returning whether this was possible without discarding any
    /// bit information
    pub fn reshape(&mut self, to: Layout) -> bool {
        match (self.layout, to) {
            // We need to change only bit dimensions
            (
                Layout::Integer(IntLayout { signed: false, bytes: len1 }),
                Layout::Integer(IntLayout { signed: false, bytes: len2 }),
            ) => {
                self.layout = to;
                self.clean();
                len1 > len2
            }
            (from, to) => unimplemented!("Number layout reshape from {} to {}", from, to),
        }
    }

    /// Adds or removes negative sign to the number (negates negative or positive number, depending
    /// on the method argument value)
    #[inline]
    pub fn applying_sign(mut self, sign: impl Into<bool>) -> Number {
        if sign.into() {
            self[self.layout.sign_byte()] |= 0x80;
        } else {
            self[self.layout.sign_byte()] &= 0x7F;
        }
        self
    }

    /// Removes negative sign if present (negates negative number)
    #[inline]
    pub fn without_sign(mut self) -> Number { self.applying_sign(false) }

    #[doc(hidden)]
    /// Converts the value into `u1024` integer with the bytes corresponding to the internal
    /// representation.
    #[inline]
    pub(super) fn to_u1024_bytes(self) -> u1024 { self.to_clean().into() }
}

/// Errors parsing literal values in AluVM assembly code
#[derive(Clone, Eq, PartialEq, Debug, Display, From)]
#[cfg_attr(feature = "std", derive(Error))]
#[display(inner)]
#[allow(clippy::branches_sharing_code)]
#[non_exhaustive]
pub enum LiteralParseError {
    /// Error parsing decimal literal
    #[from]
    Int(core::num::ParseIntError),

    /// Error parsing float value
    #[from]
    #[display(Debug)]
    Float(rustc_apfloat::ParseError),

    /// Unknown literal
    #[display("unknown token `{0}` while parsing AluVM assembly literal")]
    UnknownLiteral(String),
}

impl FromStr for Number {
    type Err = LiteralParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if s.starts_with("0x") {
            u128::from_str_radix(s, 16)?.into()
        } else if s.starts_with("0o") {
            u128::from_str_radix(s, 8)?.into()
        } else if s.starts_with("0b") {
            u128::from_str_radix(s, 2)?.into()
        } else if s.contains(&['E', 'e'][..]) {
            ieee::Double::from_str(s)?.into()
        } else if s.starts_with('-') {
            i128::from_str(s)?.into()
        } else {
            u128::from_str(s)?.into()
        })
    }
}

impl Display for Number {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.layout {
            Layout::Integer(IntLayout { signed: true, bytes }) if bytes <= 128 => {
                Display::fmt(&i128::from(self), f)
            }
            Layout::Integer(IntLayout { signed: false, bytes }) if bytes <= 128 => {
                Display::fmt(&u128::from(self), f)
            }
            Layout::Integer(IntLayout { signed: false, .. }) if self.min_bit_len() < 256 => {
                Display::fmt(&u256::from(self), f)
            }
            Layout::Integer(IntLayout { signed: false, .. }) if self.min_bit_len() < 512 => {
                Display::fmt(&u512::from(self), f)
            }
            Layout::Integer(IntLayout { .. }) => Display::fmt(&u1024::from(self), f),
            Layout::Float(FloatLayout::BFloat16) => Display::fmt(&half::bf16::from(self), f),
            Layout::Float(FloatLayout::IeeeHalf) => Display::fmt(&ieee::Half::from(self), f),
            Layout::Float(FloatLayout::IeeeSingle) => Display::fmt(&ieee::Single::from(self), f),
            Layout::Float(FloatLayout::IeeeDouble) => Display::fmt(&ieee::Double::from(self), f),
            Layout::Float(FloatLayout::IeeeQuad) => Display::fmt(&ieee::Quad::from(self), f),
            Layout::Float(FloatLayout::X87DoubleExt) => {
                Display::fmt(&ieee::X87DoubleExtended::from(self), f)
            }
            _ => {
                // TODO: Implement Display for the rest of float layouts
                f.write_str("<not supported float layout for display>")
            }
        }
    }
}

impl LowerHex for Number {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "std")]
        use amplify_num::hex::ToHex;

        match self.layout {
            Layout::Integer(IntLayout { signed: true, bytes }) if bytes <= 128 => {
                LowerHex::fmt(&i128::from(self), f)
            }
            Layout::Integer(IntLayout { signed: false, bytes }) if bytes <= 128 => {
                LowerHex::fmt(&u128::from(self), f)
            }
            // TODO: Use LowerHex implementation once it will be done in amplify_num
            Layout::Integer(IntLayout { signed: false, bytes }) if bytes < 256 => {
                #[cfg(feature = "std")]
                {
                    f.write_str(&u256::from(self).to_le_bytes().to_hex())
                }
                #[cfg(not(feature = "std"))]
                {
                    f.write_str("<hex display requires std library>")
                }
            }
            Layout::Integer(IntLayout { signed: false, bytes }) if bytes < 512 => {
                #[cfg(feature = "std")]
                {
                    f.write_str(&u512::from(self).to_le_bytes().to_hex())
                }
                #[cfg(not(feature = "std"))]
                {
                    f.write_str("<hex display requires std library>")
                }
            }
            Layout::Integer(IntLayout { .. }) => {
                #[cfg(feature = "std")]
                {
                    f.write_str(&u1024::from(self).to_le_bytes().to_hex())
                }
                #[cfg(not(feature = "std"))]
                {
                    f.write_str("<hex display requires std library>")
                }
            }
            // TODO: Use LowerHex implementation once it will be done in `half` crate
            /* TODO: Use LowerHex implementation once it will be done in `rustc_apfloat`
            Layout::Float(FloatLayout::BFloat16) => LowerHex::fmt(&half::bf16::from(self), f),
            Layout::Float(FloatLayout::IeeeHalf) => LowerHex::fmt(&ieee::Half::from(self), f),
            Layout::Float(FloatLayout::IeeeSingle) => LowerHex::fmt(&ieee::Single::from(self), f),
            Layout::Float(FloatLayout::IeeeDouble) => LowerHex::fmt(&ieee::Double::from(self), f),
            Layout::Float(FloatLayout::IeeeQuad) => LowerHex::fmt(&ieee::Quad::from(self), f),
            Layout::Float(FloatLayout::X87DoubleExt) => {
                LowerHex::fmt(&ieee::X87DoubleExtended::from(self), f)
            }
             */
            _ => {
                // TODO: Implement Display for the rest of float layouts
                f.write_str("<not supported float layout for display>")
            }
        }
    }
}

impl UpperHex for Number {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = if f.alternate() { format!("{:#x}", self) } else { format!("{:x}", self) };
        f.write_str(&s.to_uppercase())
    }
}

impl Octal for Number {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.layout {
            Layout::Integer(IntLayout { signed: true, bytes }) if bytes <= 128 => {
                Octal::fmt(&i128::from(self), f)
            }
            Layout::Integer(IntLayout { signed: false, bytes }) if bytes <= 128 => {
                Octal::fmt(&u128::from(self), f)
            }
            // TODO: Use LowerHex implementation once it will be done in amplify_num
            // TODO: Use LowerHex implementation once it will be done in `half` crate
            /* TODO: Use LowerHex implementation once it will be done in `rustc_apfloat`
            Layout::Integer(IntLayout { signed: false, bytes }) if bytes < 256 => {
                Octal::fmt(&u256::from(self), f)
            }
            Layout::Integer(IntLayout { signed: false, bytes }) if bytes < 512 => {
                Octal::fmt(&u512::from(self), f)
            }
            Layout::Integer(IntLayout { .. }) => Octal::fmt(&u1024::from(self), f),
            Layout::Float(FloatLayout::BFloat16) => Octal::fmt(&half::bf16::from(self), f),
            Layout::Float(FloatLayout::IeeeHalf) => Octal::fmt(&ieee::Half::from(self), f),
            Layout::Float(FloatLayout::IeeeSingle) => Octal::fmt(&ieee::Single::from(self), f),
            Layout::Float(FloatLayout::IeeeDouble) => Octal::fmt(&ieee::Double::from(self), f),
            Layout::Float(FloatLayout::IeeeQuad) => Octal::fmt(&ieee::Quad::from(self), f),
            Layout::Float(FloatLayout::X87DoubleExt) => {
                Octal::fmt(&ieee::X87DoubleExtended::from(self), f)
            }*/
            _ => {
                // TODO: Implement Display for the rest of float layouts
                f.write_str("<not supported float layout for display>")
            }
        }
    }
}

impl LowerExp for Number {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.layout {
            Layout::Integer(_) => Display::fmt(self, f),
            // TODO: Use LowerHex implementation once it will be done in amplify_num
            // TODO: Use LowerHex implementation once it will be done in `half` crate
            /* TODO: Use LowerHex implementation once it will be done in `rustc_apfloat`
            Layout::Float(FloatLayout::BFloat16) => LowerExp::fmt(&half::bf16::from(self), f),
            Layout::Float(FloatLayout::IeeeHalf) => LowerExp::fmt(&ieee::Half::from(self), f),
            Layout::Float(FloatLayout::IeeeSingle) => LowerExp::fmt(&ieee::Single::from(self), f),
            Layout::Float(FloatLayout::IeeeDouble) => LowerExp::fmt(&ieee::Double::from(self), f),
            Layout::Float(FloatLayout::IeeeQuad) => LowerExp::fmt(&ieee::Quad::from(self), f),
            Layout::Float(FloatLayout::X87DoubleExt) => {
                LowerExp::fmt(&ieee::X87DoubleExtended::from(self), f)
            }*/
            _ => {
                // TODO: Implement LowerExp for the rest of float layouts
                f.write_str("<not supported float layout for display>")
            }
        }
    }
}

impl UpperExp for Number {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = if f.alternate() { format!("{:#e}", self) } else { format!("{:e}", self) };
        f.write_str(&s.to_uppercase())
    }
}

macro_rules! impl_number_bytes_conv {
    ($len:literal) => {
        impl From<Number> for [u8; $len] {
            fn from(mut val: Number) -> Self {
                assert_eq!(
                    val.len(),
                    $len,
                    "attempt to convert number into a byte array with incorrect length",
                );
                let mut bytes = [0u8; $len];
                let clean = Number::default();
                val.bytes[$len..].copy_from_slice(&clean.bytes[$len..]);
                bytes.copy_from_slice(&val.bytes[0..$len]);
                bytes
            }
        }

        impl From<[u8; $len]> for Number {
            fn from(val: [u8; $len]) -> Number {
                let mut bytes = [0u8; 1024];
                bytes[0..$len].copy_from_slice(&val[..]);
                Number { layout: Layout::unsigned($len), bytes }
            }
        }

        impl From<[u8; $len]> for MaybeNumber {
            fn from(val: [u8; $len]) -> MaybeNumber { MaybeNumber::from(Number::from(val)) }
        }

        impl From<Option<[u8; $len]>> for MaybeNumber {
            fn from(val: Option<[u8; $len]>) -> MaybeNumber {
                MaybeNumber::from(val.map(Number::from))
            }
        }

        impl From<&Option<[u8; $len]>> for MaybeNumber {
            fn from(val: &Option<[u8; $len]>) -> MaybeNumber {
                MaybeNumber::from(val.map(Number::from))
            }
        }
    };
}

macro_rules! impl_number_le_conv {
    ($ty:ident, $len:literal) => {
        impl From<Number> for $ty {
            fn from(val: Number) -> Self {
                assert!(
                    val.layout.bits() as u32 <= $len,
                    "attempt to convert Number into type with lower bit dimension"
                );
                impl_number_le_from!($ty, $len, val)
            }
        }

        impl From<&$ty> for Number {
            fn from(val: &$ty) -> Self {
                let mut bytes = [0u8; 1024];
                let le = impl_number_le_into!($ty, $len, val);
                bytes[0..le.len()].copy_from_slice(&le[..]);
                Number { layout: Layout::unsigned(le.len() as u16), bytes }
            }
        }

        impl_number_ty_conv!($ty, $len);
    };
}

macro_rules! impl_number_ieee_conv {
    ($ty:ident, $tys:ident, $len:literal, $layout:ident) => {
        impl From<Number> for $ty {
            fn from(val: Number) -> Self {
                assert!(
                    val.layout.bits() as u32 <= $len,
                    "attempt to convert Number into type with lower bit dimension"
                );
                impl_number_ieee_from!($tys, $len, val)
            }
        }

        impl From<&$ty> for Number {
            fn from(val: &$ty) -> Self {
                let mut bytes = [0u8; 1024];
                let le = impl_number_ieee_into!($ty, $len, val);
                bytes[0..le.len()].copy_from_slice(&le[..]);
                Number { layout: Layout::float(FloatLayout::$layout), bytes }
            }
        }

        impl_number_ty_conv!($ty, $len);
    };
}

macro_rules! impl_number_le_from {
    ($ty:ident, $len:literal, $val:ident) => {
        $ty::from_le_bytes(<[u8; $len]>::from($val))
    };
}

macro_rules! impl_number_ieee_from {
    ($ty:ident, $len:literal, $val:ident) => {
        $ty::from_bits($val.into())
    };
}

macro_rules! impl_number_le_into {
    ($ty:ident, $len:literal, $val:ident) => {
        $val.to_le_bytes()
    };
}

macro_rules! impl_number_ieee_into {
    ($ty:ident, $len:literal, $val:ident) => {
        $val.to_bits().to_le_bytes()
    };
}

macro_rules! impl_number_ty_conv {
    ($ty:ident, $len:literal) => {
        impl From<&Number> for $ty {
            fn from(val: &Number) -> Self { $ty::from(*val) }
        }

        impl From<$ty> for Number {
            fn from(val: $ty) -> Self { Number::from(&val) }
        }

        impl From<$ty> for MaybeNumber {
            fn from(val: $ty) -> Self { MaybeNumber::some(Number::from(val)) }
        }
        impl From<&$ty> for MaybeNumber {
            fn from(val: &$ty) -> Self { MaybeNumber::some(Number::from(*val)) }
        }
        impl From<Option<$ty>> for MaybeNumber {
            fn from(val: Option<$ty>) -> Self { MaybeNumber::from(val.map(Number::from)) }
        }
        impl From<Option<&$ty>> for MaybeNumber {
            fn from(val: Option<&$ty>) -> Self { MaybeNumber::from(val.copied().map(Number::from)) }
        }
        impl From<&Option<$ty>> for MaybeNumber {
            fn from(val: &Option<$ty>) -> Self { MaybeNumber::from((*val).map(Number::from)) }
        }
    };
}

impl_number_bytes_conv!(1);
impl_number_bytes_conv!(2);
impl_number_bytes_conv!(4);
impl_number_bytes_conv!(8);
impl_number_bytes_conv!(16);
impl_number_bytes_conv!(20);
impl_number_bytes_conv!(32);
impl_number_bytes_conv!(64);
impl_number_bytes_conv!(128);
impl_number_bytes_conv!(256);
impl_number_bytes_conv!(512);
impl_number_bytes_conv!(1024);

impl_number_le_conv!(u8, 1);
impl_number_le_conv!(u16, 2);
impl_number_le_conv!(u32, 4);
impl_number_le_conv!(u64, 8);
impl_number_le_conv!(u128, 16);
impl_number_le_conv!(u256, 32);
impl_number_le_conv!(u512, 64);
impl_number_le_conv!(u1024, 128);

mod _float_impl {
    use half::bf16;
    use rustc_apfloat::ieee::*;
    use rustc_apfloat::Float;

    use super::*;

    impl_number_ieee_conv!(bf16, bf16, 2, BFloat16);
    impl_number_ieee_conv!(Half, HalfS, 2, IeeeHalf);
    impl_number_ieee_conv!(Single, SingleS, 4, IeeeSingle);
    impl_number_ieee_conv!(Double, DoubleS, 8, IeeeDouble);
    impl_number_ieee_conv!(X87DoubleExtended, X87DoubleExtendedS, 10, X87DoubleExt);
    impl_number_ieee_conv!(Quad, QuadS, 16, IeeeQuad);
}

impl_number_le_conv!(i8, 1);
impl_number_le_conv!(i16, 2);
impl_number_le_conv!(i32, 4);
impl_number_le_conv!(i64, 8);
impl_number_le_conv!(i128, 16);

/// Value for step instructions which can be displayed as a part of operation mnemonic
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default, From)]
pub struct Step(#[from] i16);

impl Step {
    /// Returns step value
    pub fn as_i16(self) -> i16 { self.0 }
}

impl Display for Step {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            let val = self.0;
            match val {
                1 => f.write_str("inc"),
                -1 => f.write_str("dec"),
                x if x < 0 => f.write_str("sub"),
                x if x >= 0 => f.write_str("add"),
            }
        } else {
            f.write_char(',')?;
            Display::fmt(&self.0, f)
        }
    }
}
