// 64-bit FNV-1a hashing as the macOS app does it (WordPresentation.swift,
// LyricTypography.swift, NowPlayingView.trackSeed).
//
// The macOS code starts from 1469598103934665603 — one digit short of the
// textbook FNV offset basis — and that exact value is kept so every word,
// font pick and motion phase matches the Mac byte for byte.
//
// JavaScript numbers cannot hold 64-bit integers, so a hash is a pair of
// unsigned 32-bit halves. Hashing and `mod` never allocate; `div` and `xor`
// return a new pair (used once per word when a line is prepared).

const TWO_32 = 4294967296;

export class Seed64 {
  /** hi and lo are unsigned 32-bit integers. */
  constructor(readonly hi: number, readonly lo: number) {}

  static fromBigInt(value: bigint): Seed64 {
    const v = BigInt.asUintN(64, value);
    return new Seed64(Number(v >> 32n), Number(v & 0xffffffffn));
  }

  static fromString(decimal: string): Seed64 {
    return Seed64.fromBigInt(BigInt(decimal));
  }

  /** A small non-negative integer (< 2^53). */
  static fromNumber(value: number): Seed64 {
    const v = Math.max(0, Math.floor(value));
    return new Seed64(Math.floor(v / TWO_32) >>> 0, v >>> 0);
  }

  toBigInt(): bigint {
    return (BigInt(this.hi) << 32n) | BigInt(this.lo);
  }

  toString(): string {
    return this.toBigInt().toString();
  }

  equals(other: Seed64): boolean {
    return this.hi === other.hi && this.lo === other.lo;
  }

  /** UInt64 `self % n` for an integer 1 <= n < 2^21. Allocation-free. */
  mod(n: number): number {
    return ((this.hi % n) * TWO_32 + this.lo) % n;
  }

  /** UInt64 `self / n` (truncating) for an integer 1 <= n < 2^21. */
  div(n: number): Seed64 {
    const hiQuotient = Math.floor(this.hi / n);
    const rest = (this.hi % n) * TWO_32 + this.lo; // < n * 2^32 <= 2^53: exact
    return new Seed64(hiQuotient >>> 0, Math.floor(rest / n) >>> 0);
  }

  xor(other: Seed64): Seed64 {
    return new Seed64((this.hi ^ other.hi) >>> 0, (this.lo ^ other.lo) >>> 0);
  }

  /** The value as a double (exact below 2^53). Used for `Int(value / k) % m` style maths. */
  toNumber(): number {
    return this.hi * TWO_32 + this.lo;
  }
}

/** The macOS basis, 1469598103934665603 = 0x14650FB0_739D0383. */
export const FNV_BASIS = Seed64.fromBigInt(1469598103934665603n);

const encoder = new TextEncoder();

/** FNV-1a over bytes, continuing from `basis`. Prime 1099511628211 = 2^40 + 435. */
export function fnvBytes(bytes: Uint8Array, basis: Seed64 = FNV_BASIS): Seed64 {
  let hi = basis.hi;
  let lo = basis.lo;
  for (let i = 0; i < bytes.length; i++) {
    lo = (lo ^ bytes[i]) >>> 0;
    const low = lo * 435; // < 2^41, exact
    const carry = Math.floor(low / TWO_32);
    const nextHi = (hi * 435 + lo * 256 + carry) % TWO_32; // < 2^42, exact
    lo = low % TWO_32;
    hi = nextHi;
  }
  return new Seed64(hi >>> 0, lo >>> 0);
}

/** FNV-1a over the UTF-8 bytes of `text` (Swift `text.utf8.reduce`). */
export function fnvString(text: string, basis: Seed64 = FNV_BASIS): Seed64 {
  return fnvBytes(encoder.encode(text), basis);
}

const scratch = new DataView(new ArrayBuffer(8));

/** IEEE-754 bit pattern of a double (Swift `Double.bitPattern`). */
export function doubleBits(value: number): Seed64 {
  scratch.setFloat64(0, value, false);
  return new Seed64(scratch.getUint32(0, false), scratch.getUint32(4, false));
}

/** `TimedWordKey.visualSeed`: FNV-1a over the text, seeded with the start's bits. */
export function visualSeed(text: string, start: number): Seed64 {
  return fnvString(text, doubleBits(start).xor(FNV_BASIS));
}

/** Stable identity of a timed word across line windows (`TimedWordKey`). */
export function timedWordKey(text: string, start: number, end: number): string {
  return `${start}|${end}|${text}`;
}

/** macOS `NowPlayingView.trackSeed`: FNV-1a of title + artist, mod 100000. */
export function trackSeed(title: string, artist: string): number {
  return fnvString(title + artist).mod(100_000);
}
