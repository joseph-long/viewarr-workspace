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
