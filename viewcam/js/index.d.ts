/**
 * TypeScript type definitions for viewcam
 */

/** Settings reported to {@link onStateChange}. */
export interface ViewcamState {
  darkSubtraction: boolean;
  saturationOverlay: boolean;
  /** CSS `#rrggbb`. */
  saturationColor: string;
  /** The status bar's turbo toggle. */
  turbo: boolean;
}

export type FeedRole = 'data' | 'dark' | 'saturation';
export type FeedConnection = 'connecting' | 'open' | 'closed';

/** Create a new camera viewer in the element with this ID. */
export function createViewer(containerId: string): Promise<void>;

/** Set the science frame (dark-subtracted while that toggle is on). */
export function setData(
  containerId: string,
  buffer: ArrayBuffer,
  width: number,
  height: number,
  dtype: string
): void;

/** Set the dark frame; kept until replaced or cleared. */
export function setDark(
  containerId: string,
  buffer: ArrayBuffer,
  width: number,
  height: number,
  dtype: string
): void;

export function clearDark(containerId: string): void;

/** Set the saturation image; nonzero pixels are painted in the saturation colour. */
export function setSaturation(
  containerId: string,
  buffer: ArrayBuffer,
  width: number,
  height: number,
  dtype: string
): void;

/** Remove the saturation image, or the saturation level. */
export function clearSaturation(containerId: string): void;

/**
 * For a camera without a saturation image: paint each data frame's pixels at
 * or above `level` (in the data's units, before dark subtraction). Replaces
 * any saturation image, as {@link setSaturation} replaces the level.
 */
export function setSaturationLevel(containerId: string, level: number): void;

export function getDarkSubtraction(containerId: string): boolean;
export function setDarkSubtraction(containerId: string, enabled: boolean): void;

export function getSaturationOverlay(containerId: string): boolean;
export function setSaturationOverlay(containerId: string, enabled: boolean): void;

/** CSS `#rrggbb`. */
export function getSaturationColor(containerId: string): string;
/** CSS `#rrggbb`; throws on anything else. */
export function setSaturationColor(containerId: string, color: string): void;

/** Name a feed's stream in the status bar, as connecting; null takes it out. */
export function setFeed(containerId: string, role: FeedRole, name: string | null): void;
/** A feed's connection changed; its frame rate starts over. */
export function setFeedConnection(containerId: string, role: FeedRole, connection: FeedConnection): void;
/** A feed's frame was shown. */
export function noteFeedFrame(
  containerId: string,
  role: FeedRole,
  seq: bigint | number,
  width: number,
  height: number,
  dtype: string
): void;
/** A feed said it has no image. */
export function noteFeedEmpty(containerId: string, role: FeedRole): void;
/** A message of `bytes` arrived on any feed, for the bandwidth. */
export function noteReceived(containerId: string, bytes: number): void;

/** The status bar's turbo toggle; what it does is up to the caller. */
export function getTurbo(containerId: string): boolean;
export function setTurbo(containerId: string, turbo: boolean): void;

export function onStateChange(containerId: string, callback: (state: ViewcamState) => void): void;
export function clearCallbacks(containerId: string): void;

export function destroyViewer(containerId: string): void;
export function hasViewer(containerId: string): boolean;

declare const viewcam: {
  createViewer: typeof createViewer;
  setData: typeof setData;
  setDark: typeof setDark;
  clearDark: typeof clearDark;
  setSaturation: typeof setSaturation;
  clearSaturation: typeof clearSaturation;
  setSaturationLevel: typeof setSaturationLevel;
  getDarkSubtraction: typeof getDarkSubtraction;
  setDarkSubtraction: typeof setDarkSubtraction;
  getSaturationOverlay: typeof getSaturationOverlay;
  setSaturationOverlay: typeof setSaturationOverlay;
  getSaturationColor: typeof getSaturationColor;
  setSaturationColor: typeof setSaturationColor;
  setFeed: typeof setFeed;
  setFeedConnection: typeof setFeedConnection;
  noteFeedFrame: typeof noteFeedFrame;
  noteFeedEmpty: typeof noteFeedEmpty;
  noteReceived: typeof noteReceived;
  getTurbo: typeof getTurbo;
  setTurbo: typeof setTurbo;
  onStateChange: typeof onStateChange;
  clearCallbacks: typeof clearCallbacks;
  destroyViewer: typeof destroyViewer;
  hasViewer: typeof hasViewer;
};

export default viewcam;
