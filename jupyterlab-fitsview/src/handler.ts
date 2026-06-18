import { URLExt } from '@jupyterlab/coreutils';
import { ServerConnection } from '@jupyterlab/services';

/**
 * Enum mapping Rust-style type specifiers to TypedArray constructors.
 * Values match the Python ArrayType enum in handlers.py.
 */
export enum ArrayType {
  INT8 = 'i8',
  UINT8 = 'u8',
  INT16 = 'i16',
  UINT16 = 'u16',
  INT32 = 'i32',
  UINT32 = 'u32',
  INT64 = 'i64',
  UINT64 = 'u64',
  FLOAT32 = 'f32',
  FLOAT64 = 'f64'
}

/**
 * Call the FITS API extension
 *
 * @param endPoint API REST end point for the extension
 * @param init Initial values for the request
 * @returns The response body interpreted as JSON
 */
export async function requestAPI<T>(
  endPoint = '',
  init: RequestInit = {}
): Promise<T> {
  const settings = ServerConnection.makeSettings();
  const requestUrl = URLExt.join(
    settings.baseUrl,
    'fitsview', // API Namespace
    endPoint
  );

  let response: Response;
  try {
    response = await ServerConnection.makeRequest(requestUrl, init, settings);
  } catch (error) {
    throw new ServerConnection.NetworkError(error as any);
  }

  let data: any = await response.text();

  if (data.length > 0) {
    try {
      data = JSON.parse(data);
    } catch (error) {
      console.log('Not a JSON response body.', response);
    }
  }

  if (!response.ok) {
    // Extract error message from various response formats
    let errorMessage = 'Unknown error';
    if (typeof data === 'string') {
      errorMessage = data;
    } else if (data && typeof data === 'object') {
      errorMessage = data.error || data.message || JSON.stringify(data);
    }
    throw new ServerConnection.ResponseError(response, errorMessage);
  }

  return data;
}

/**
 * Call the FITS API extension and return binary data
 *
 * @param endPoint API REST end point for the extension
 * @param init Initial values for the request
 * @returns The response body as ArrayBuffer, shape, and arrayType from headers
 */
export async function requestBinaryAPI(
  endPoint = '',
  init: RequestInit = {}
): Promise<{ buffer: ArrayBuffer; shape: number[]; arrayType: ArrayType }> {
  const settings = ServerConnection.makeSettings();
  const requestUrl = URLExt.join(
    settings.baseUrl,
    'fitsview', // API Namespace
    endPoint
  );

  let response: Response;
  try {
    response = await ServerConnection.makeRequest(requestUrl, init, settings);
  } catch (error) {
    throw new ServerConnection.NetworkError(error as any);
  }

  if (!response.ok) {
    const text = await response.text();
    let message = text;
    try {
      const json = JSON.parse(text);
      message = json.error || json.message || text;
    } catch {
      // Not JSON, use text as-is
    }
    throw new ServerConnection.ResponseError(response, message);
  }

  const buffer = await response.arrayBuffer();
  const shapeHeader = response.headers.get('X-FITS-Shape');
  const shape = shapeHeader ? JSON.parse(shapeHeader) : [];
  const typeHeader = response.headers.get('X-FITS-Type');
  const arrayType = (typeHeader as ArrayType) || ArrayType.FLOAT64;

  return { buffer, shape, arrayType };
}

/**
 * TypedArray type union for all supported array types
 */
export type TypedArray =
  | Int8Array
  | Uint8Array
  | Int16Array
  | Uint16Array
  | Int32Array
  | Uint32Array
  | Float32Array
  | Float64Array
  | BigInt64Array
  | BigUint64Array;

/**
 * Get the byte size per element for a given ArrayType
 */
export function getArrayTypeByteSize(arrayType: ArrayType): number {
  switch (arrayType) {
    case ArrayType.INT8:
    case ArrayType.UINT8:
      return 1;
    case ArrayType.INT16:
    case ArrayType.UINT16:
      return 2;
    case ArrayType.INT32:
    case ArrayType.UINT32:
    case ArrayType.FLOAT32:
      return 4;
    case ArrayType.INT64:
    case ArrayType.UINT64:
    case ArrayType.FLOAT64:
    default:
      return 8;
  }
}

/**
 * Calculate the byte size of a 2D image slice from shape and arrayType
 */
export function calculateSliceByteSize(
  shape: number[],
  arrayType: ArrayType
): number {
  if (shape.length < 2) {
    return 0;
  }
  // Last two dimensions are the image plane
  const height = shape[shape.length - 2];
  const width = shape[shape.length - 1];
  const numPixels = width * height;
  return numPixels * getArrayTypeByteSize(arrayType);
}

/**
 * Format byte size as human-readable string
 */
export function formatByteSize(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  } else if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  } else if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  } else {
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
}

/**
 * Callback for progress updates during fetch
 */
export type ProgressCallback = (loaded: number, total: number) => void;

/**
 * Call the FITS API extension and return binary data with progress tracking
 * and cancellation support.
 *
 * @param endPoint API REST end point for the extension
 * @param onProgress Optional callback for progress updates
 * @param signal Optional AbortSignal for cancellation
 * @returns The response body as ArrayBuffer, shape, and arrayType from headers
 */
export async function requestBinaryAPIWithProgress(
  endPoint: string,
  onProgress?: ProgressCallback,
  signal?: AbortSignal
): Promise<{ buffer: ArrayBuffer; shape: number[]; arrayType: ArrayType }> {
  const settings = ServerConnection.makeSettings();
  const requestUrl = URLExt.join(settings.baseUrl, 'fitsview', endPoint);

  // Build headers from settings
  const headers: HeadersInit = {};
  if (settings.token) {
    headers['Authorization'] = `token ${settings.token}`;
  }

  const response = await fetch(requestUrl, {
    method: 'GET',
    credentials: 'same-origin',
    headers,
    signal
  });

  if (!response.ok) {
    const text = await response.text();
    let message = text;
    try {
      const json = JSON.parse(text);
      message = json.error || json.message || text;
    } catch {
      // Not JSON, use text as-is
    }
    throw new ServerConnection.ResponseError(response, message);
  }

  const shapeHeader = response.headers.get('X-FITS-Shape');
  const shape = shapeHeader ? JSON.parse(shapeHeader) : [];
  const typeHeader = response.headers.get('X-FITS-Type');
  const arrayType = (typeHeader as ArrayType) || ArrayType.FLOAT64;
  const contentLength = response.headers.get('Content-Length');
  const total = contentLength ? parseInt(contentLength, 10) : 0;

  // If no progress callback or no content length, just get the buffer directly
  if (!onProgress || !total || !response.body) {
    const buffer = await response.arrayBuffer();
    return { buffer, shape, arrayType };
  }

  // Stream the response with progress tracking
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let loaded = 0;

  let readResult = await reader.read();
  while (!readResult.done) {
    chunks.push(readResult.value);
    loaded += readResult.value.length;
    onProgress(loaded, total);
    readResult = await reader.read();
  }

  // Combine chunks into a single ArrayBuffer
  const buffer = new ArrayBuffer(loaded);
  const view = new Uint8Array(buffer);
  let offset = 0;
  for (const chunk of chunks) {
    view.set(chunk, offset);
    offset += chunk.length;
  }

  return { buffer, shape, arrayType };
}

/**
 * Create a TypedArray from an ArrayBuffer based on ArrayType enum.
 * Assumes little-endian byte order (server converts to LE before sending).
 */
export function createTypedArray(
  buffer: ArrayBuffer,
  arrayType: ArrayType
): TypedArray {
  switch (arrayType) {
    case ArrayType.INT8:
      return new Int8Array(buffer);
    case ArrayType.UINT8:
      return new Uint8Array(buffer);
    case ArrayType.INT16:
      return new Int16Array(buffer);
    case ArrayType.UINT16:
      return new Uint16Array(buffer);
    case ArrayType.INT32:
      return new Int32Array(buffer);
    case ArrayType.UINT32:
      return new Uint32Array(buffer);
    case ArrayType.INT64:
      return new BigInt64Array(buffer);
    case ArrayType.UINT64:
      return new BigUint64Array(buffer);
    case ArrayType.FLOAT32:
      return new Float32Array(buffer);
    case ArrayType.FLOAT64:
    default:
      return new Float64Array(buffer);
  }
}
