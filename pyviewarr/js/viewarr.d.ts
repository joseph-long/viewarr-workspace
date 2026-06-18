declare module "viewarr" {
	export type StretchMode = "linear" | "log" | "symmetric";

	export interface ViewerStateConfig {
		contrast?: number;
		bias?: number;
		stretchMode?: StretchMode;
		zoom?: number;
		colormap?: string;
		colormapReversed?: boolean;
		vmin?: number;
		vmax?: number;
		xlim?: [number, number];
		ylim?: [number, number];
		rotation?: number;
		pivot?: [number, number];
		showPivotMarker?: boolean;
		overlayMessage?: string;
		markers?: [number, number][];
	}

	export interface ViewerState {
		contrast: number;
		bias: number;
		stretchMode: StretchMode;
		zoom: number;
		colormap: string;
		colormapReversed: boolean;
		vmin: number;
		vmax: number;
		xlim?: [number, number];
		ylim?: [number, number];
		rotation: number;
		pivot: [number, number];
		showPivotMarker: boolean;
	}

	/**
	 * Create a new viewer instance in the specified container.
	 * @param containerId - The ID of the HTML element to use as the container.
	 * @returns Promise that resolves when the viewer is ready.
	 */
	export function createViewer(containerId: string): Promise<void>;

	/**
	 * Set image data for a viewer.
	 * @param containerId - The ID of the container (viewer instance).
	 * @param buffer - The raw pixel data.
	 * @param width - Image width in pixels.
	 * @param height - Image height in pixels.
	 * @param dtype - Data type string (e.g., "f32", "f64", "i16", "u8").
	 */
	export function setImageData(
		containerId: string,
		buffer: ArrayBuffer,
		width: number,
		height: number,
		dtype: string
	): void;

	/**
	 * Destroy a viewer instance and clean up resources.
	 * @param containerId - The ID of the container (viewer instance).
	 */
	export function destroyViewer(containerId: string): void;

	/**
	 * Check if a viewer exists for a container.
	 */
	export function hasViewer(containerId: string): boolean;
	export function getContrast(containerId: string): number;
	export function setContrast(containerId: string, contrast: number): void;
	export function getBias(containerId: string): number;
	export function setBias(containerId: string, bias: number): void;
	export function getStretchMode(containerId: string): StretchMode;
	export function setStretchMode(containerId: string, mode: StretchMode): void;
	export function getZoom(containerId: string): number;
	export function setZoom(containerId: string, zoom: number): void;
	export function getViewBounds(containerId: string): [number, number, number, number];
	export function setViewBounds(
		containerId: string,
		xmin: number,
		xmax: number,
		ymin: number,
		ymax: number
	): void;
	export function getColormap(containerId: string): string;
	export function getColormapReversed(containerId: string): boolean;
	export function getValueRange(containerId: string): [number, number];
	export function getRotation(containerId: string): number;
	export function setRotation(containerId: string, degrees: number): void;
	export function getPivotPoint(containerId: string): [number, number];
	export function setPivotPoint(containerId: string, x: number, y: number): void;
	export function getShowPivotMarker(containerId: string): boolean;
	export function setShowPivotMarker(containerId: string, show: boolean): void;
	export function getOverlayMessage(containerId: string): string;
	export function setOverlayMessage(containerId: string, message: string): void;
	export function getMarkers(containerId: string): [number, number][];
	export function setMarkers(
		containerId: string,
		markers: [number, number][]
	): void;
	export interface ClickEvent {
		x: number;
		y: number;
	}
	export function onStateChange(
		containerId: string,
		callback: (state: ViewerState) => void
	): void;
	export function onClick(
		containerId: string,
		callback: (event: ClickEvent) => void
	): void;
	export function clearCallbacks(containerId: string): void;
	export function setViewerState(containerId: string, state: ViewerStateConfig): void;
}
