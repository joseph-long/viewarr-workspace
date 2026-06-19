// Type declarations for viewarr WASM module
declare module 'viewarr' {
  /**
   * Create a new viewer instance in the specified container.
   */
  export function createViewer(containerId: string): Promise<void>;

  /**
   * Set image data for a viewer.
   * Pan is reset if dimensions change; zoom is always preserved.
   */
  export function setImageData(
    containerId: string,
    buffer: ArrayBuffer,
    width: number,
    height: number,
    dtype: string
  ): void;

  /**
   * Declare the sliceable leading axes of an N-D cube. The widget renders the
   * slice + play controls; pass an empty array for a plain 2D image.
   * @param dims - Lengths of the leading (sliceable) axes, outer→inner.
   */
  export function setCube(containerId: string, dims: number[]): void;

  /**
   * Set image data for a specific cube slice, tagged with its slice indices so
   * the widget can sync slider positions and correlate play-mode prefetches.
   * @param indices - Slice indices this image corresponds to (may be empty).
   */
  export function setSliceData(
    containerId: string,
    buffer: ArrayBuffer,
    width: number,
    height: number,
    dtype: string,
    indices: number[]
  ): void;

  /**
   * Register a callback invoked when the widget needs a cube slice fetched
   * (slider drag or play loop). The host should fetch that slice and deliver it
   * via setSliceData.
   */
  export function onSliceRequest(
    containerId: string,
    callback: (indices: number[]) => void
  ): void;

  /**
   * Notify a viewer that its container has been resized.
   */
  export function notifyResize(
    containerId: string,
    width: number,
    height: number
  ): void;

  /**
   * Destroy a viewer instance and clean up resources.
   */
  export function destroyViewer(containerId: string): void;

  /**
   * Check if a viewer exists for a given container.
   */
  export function hasViewer(containerId: string): boolean;

  /**
   * Get all active viewer IDs.
   */
  export function getActiveViewers(): string[];
}
