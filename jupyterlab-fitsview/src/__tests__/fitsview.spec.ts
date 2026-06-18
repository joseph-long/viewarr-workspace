/**
 * Unit tests for fitsview extension
 */

import { ArrayType, createTypedArray, getArrayTypeByteSize } from '../handler';

describe('createTypedArray', () => {
  it('should create Float32Array for f32 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.FLOAT32);
    expect(result).toBeInstanceOf(Float32Array);
    expect(result.length).toBe(4);
  });

  it('should create Float64Array for f64 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.FLOAT64);
    expect(result).toBeInstanceOf(Float64Array);
    expect(result.length).toBe(2);
  });

  it('should create Int8Array for i8 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.INT8);
    expect(result).toBeInstanceOf(Int8Array);
    expect(result.length).toBe(16);
  });

  it('should create Uint8Array for u8 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.UINT8);
    expect(result).toBeInstanceOf(Uint8Array);
    expect(result.length).toBe(16);
  });

  it('should create Int16Array for i16 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.INT16);
    expect(result).toBeInstanceOf(Int16Array);
    expect(result.length).toBe(8);
  });

  it('should create Uint16Array for u16 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.UINT16);
    expect(result).toBeInstanceOf(Uint16Array);
    expect(result.length).toBe(8);
  });

  it('should create Int32Array for i32 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.INT32);
    expect(result).toBeInstanceOf(Int32Array);
    expect(result.length).toBe(4);
  });

  it('should create Uint32Array for u32 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.UINT32);
    expect(result).toBeInstanceOf(Uint32Array);
    expect(result.length).toBe(4);
  });

  it('should create BigInt64Array for i64 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.INT64);
    expect(result).toBeInstanceOf(BigInt64Array);
    expect(result.length).toBe(2);
  });

  it('should create BigUint64Array for u64 type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, ArrayType.UINT64);
    expect(result).toBeInstanceOf(BigUint64Array);
    expect(result.length).toBe(2);
  });

  it('should default to Float64Array for unknown type', () => {
    const buffer = new ArrayBuffer(16);
    const result = createTypedArray(buffer, 'unknown' as ArrayType);
    expect(result).toBeInstanceOf(Float64Array);
  });
});

describe('getArrayTypeByteSize', () => {
  it('should return 1 for 8-bit types', () => {
    expect(getArrayTypeByteSize(ArrayType.INT8)).toBe(1);
    expect(getArrayTypeByteSize(ArrayType.UINT8)).toBe(1);
  });

  it('should return 2 for 16-bit types', () => {
    expect(getArrayTypeByteSize(ArrayType.INT16)).toBe(2);
    expect(getArrayTypeByteSize(ArrayType.UINT16)).toBe(2);
  });

  it('should return 4 for 32-bit types', () => {
    expect(getArrayTypeByteSize(ArrayType.INT32)).toBe(4);
    expect(getArrayTypeByteSize(ArrayType.UINT32)).toBe(4);
    expect(getArrayTypeByteSize(ArrayType.FLOAT32)).toBe(4);
  });

  it('should return 8 for 64-bit types', () => {
    expect(getArrayTypeByteSize(ArrayType.INT64)).toBe(8);
    expect(getArrayTypeByteSize(ArrayType.UINT64)).toBe(8);
    expect(getArrayTypeByteSize(ArrayType.FLOAT64)).toBe(8);
  });
});
