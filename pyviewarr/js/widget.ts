import type { RenderProps } from "@anywidget/types";
import {
	createViewer,
	setCube,
	setSliceData,
	destroyViewer,
	getContrast,
	setContrast,
	getBias,
	setBias,
	getStretchMode,
	setStretchMode,
	getViewBounds,
	setViewBounds,
	getZoom,
	setZoom,
	setViewerState,
	getColormap,
	getColormapReversed,
	getValueRange,
	getRotation,
	setRotation,
	getPivotPoint,
	setPivotPoint,
	getShowPivotMarker,
	setShowPivotMarker,
	setOverlayMessage,
	getMarkers,
	setMarkers,
	onStateChange,
	onClick,
	onSliceRequest,
	clearCallbacks,
	type ViewerState,
	type ClickEvent,
	type ViewerStateConfig,
	type StretchMode
} from "viewarr";
import "./widget.css";

/* Specifies attributes defined with traitlets in ../src/pyviewarr/__init__.py */
interface WidgetModel {
	data: DataView;
	image_width: number;
	image_height: number;
	_image_update_token: number;
	dtype: string;
	widget_width: number;
	widget_height: number;
	shape: number[];
	current_slice_indices: number[];
	viewer_config: ViewerStateConfig;
	overlay_message: string;
	_shift_click_event: { x: number; y: number; token: number } | Record<string, never>;
	// Viewer state properties (bidirectional sync)
	contrast: number;
	bias: number;
	stretch_mode: StretchMode;
	zoom: number;
	xlim: [number, number];
	ylim: [number, number];
	colormap: string;
	colormap_reversed: boolean;
	vmin: number;
	vmax: number;
	rotation: number;
	pivot: [number, number];
	show_pivot_marker: boolean;
	markers: [number, number][];
	_sync_from_viewer: boolean;
}

/**
 * Generate a UUID v4 for unique viewer identification.
 */
function generateUUID(): string {
	return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
		const r = (Math.random() * 16) | 0;
		const v = c === "x" ? r : (r & 0x3) | 0x8;
		return v.toString(16);
	});
}

/**
 * Wait for an element to be connected to the document DOM.
 * Returns immediately if already connected.
 */
function waitForDOMConnection(element: HTMLElement): Promise<void> {
	return new Promise((resolve) => {
		if (element.isConnected) {
			resolve();
			return;
		}

		// Use MutationObserver to detect when the element is added to the DOM
		const observer = new MutationObserver(() => {
			if (element.isConnected) {
				observer.disconnect();
				resolve();
			}
		});

		// Observe the document body for subtree additions
		observer.observe(document.body, { childList: true, subtree: true });

		// Also use requestAnimationFrame as a fallback for the next frame
		requestAnimationFrame(function checkConnection() {
			if (element.isConnected) {
				observer.disconnect();
				resolve();
			} else {
				requestAnimationFrame(checkConnection);
			}
		});
	});
}

function render({ model, el }: RenderProps<WidgetModel>) {
	const viewerId = `pyviewarr-${generateUUID()}`;

	// Create container element
	const container = document.createElement("div");
	container.id = viewerId;
	container.classList.add("pyviewarr-container");
	container.style.width = `${model.get("widget_width")}px`;
	container.style.height = `${model.get("widget_height")}px`;

	el.classList.add("pyviewarr");
	el.appendChild(container);

	let viewerReady = false;
	let isDisposed = false;
	let updatingFromViewer = false;  // Guard against feedback loops
	let lastAppliedImageToken = -1;

	/**
	 * Handle state change callback from the Rust viewer.
	 * This is called by the viewer whenever its state changes (zoom, pan, contrast, etc.)
	 */
	function handleViewerStateChange(state: ViewerState): void {
		if (!viewerReady || isDisposed || updatingFromViewer) return;

		// Set guard to prevent model changes from triggering viewer updates
		updatingFromViewer = true;

		try {
			let changed = false;

			if (Math.abs(model.get("contrast") - state.contrast) > 0.001) {
				model.set("contrast", state.contrast);
				changed = true;
			}
			if (Math.abs(model.get("bias") - state.bias) > 0.001) {
				model.set("bias", state.bias);
				changed = true;
			}
			if (model.get("stretch_mode") !== state.stretchMode) {
				model.set("stretch_mode", state.stretchMode);
				changed = true;
			}
			if (Math.abs(model.get("zoom") - state.zoom) > 0.001) {
				model.set("zoom", state.zoom);
				changed = true;
			}

			if (state.xlim && state.ylim) {
				const currentXlim = model.get("xlim") as [number, number];
				const currentYlim = model.get("ylim") as [number, number];
				if (
					Math.abs(currentXlim[0] - state.xlim[0]) > 0.5 ||
					Math.abs(currentXlim[1] - state.xlim[1]) > 0.5
				) {
					model.set("xlim", [state.xlim[0], state.xlim[1]] as [number, number]);
					changed = true;
				}
				if (
					Math.abs(currentYlim[0] - state.ylim[0]) > 0.5 ||
					Math.abs(currentYlim[1] - state.ylim[1]) > 0.5
				) {
					model.set("ylim", [state.ylim[0], state.ylim[1]] as [number, number]);
					changed = true;
				}
			}

			if (model.get("colormap") !== state.colormap) {
				model.set("colormap", state.colormap);
				changed = true;
			}
			if (model.get("colormap_reversed") !== state.colormapReversed) {
				model.set("colormap_reversed", state.colormapReversed);
				changed = true;
			}
			if (Math.abs(model.get("vmin") - state.vmin) > 1e-10) {
				model.set("vmin", state.vmin);
				changed = true;
			}
			if (Math.abs(model.get("vmax") - state.vmax) > 1e-10) {
				model.set("vmax", state.vmax);
				changed = true;
			}

			// Rotation state
			if (Math.abs(model.get("rotation") - state.rotation) > 0.01) {
				model.set("rotation", state.rotation);
				changed = true;
			}
			if (state.pivot) {
				const currentPivot = model.get("pivot") as [number, number];
				if (
					Math.abs(currentPivot[0] - state.pivot[0]) > 0.01 ||
					Math.abs(currentPivot[1] - state.pivot[1]) > 0.01
				) {
					model.set("pivot", [state.pivot[0], state.pivot[1]] as [number, number]);
					changed = true;
				}
			}
			if (model.get("show_pivot_marker") !== state.showPivotMarker) {
				model.set("show_pivot_marker", state.showPivotMarker);
				changed = true;
			}

			if (changed) {
				model.set("_sync_from_viewer", true);
				model.save_changes();
			}
		} finally {
			updatingFromViewer = false;
		}
	}

	/**
	 * Handle shift-click callback from the Rust viewer.
	 * This forwards continuous data-space coordinates to Python.
	 */
	function handleShiftClick(event: ClickEvent): void {
		if (!viewerReady || isDisposed) return;
		if (typeof event.x !== "number" || typeof event.y !== "number") return;
		model.set("_shift_click_event", {
			x: event.x,
			y: event.y,
			token: Date.now()
		});
		model.save_changes();
	}

	/**
	 * Perform an initial sync from viewer to model to capture default values.
	 */
	function initialSyncViewerToModel(): void {
		if (!viewerReady || isDisposed) return;

		try {
			const contrast = getContrast(viewerId);
			const bias = getBias(viewerId);
			const stretchMode = getStretchMode(viewerId);
			const zoom = getZoom(viewerId);
			const bounds = getViewBounds(viewerId);
			const colormap = getColormap(viewerId);
			const colormapReversed = getColormapReversed(viewerId);
			const valueRange = getValueRange(viewerId);
			const rotation = getRotation(viewerId);
			const pivot = getPivotPoint(viewerId);
			const showPivotMarker = getShowPivotMarker(viewerId);
			const markers = getMarkers(viewerId);

			model.set("contrast", contrast);
			model.set("bias", bias);
			model.set("stretch_mode", stretchMode);
			model.set("zoom", zoom);
			model.set("xlim", [bounds[0], bounds[1]] as [number, number]);
			model.set("ylim", [bounds[2], bounds[3]] as [number, number]);
			model.set("colormap", colormap);
			model.set("colormap_reversed", colormapReversed);
			model.set("vmin", valueRange[0]);
			model.set("vmax", valueRange[1]);
			model.set("rotation", rotation);
			model.set("pivot", [pivot[0], pivot[1]] as [number, number]);
			model.set("show_pivot_marker", showPivotMarker);
			model.set("markers", markers as [number, number][]);
			model.set("_sync_from_viewer", true);
			model.save_changes();
		} catch (e) {
			// Viewer may not be ready yet
		}
	}

	/**
	 * Apply model values to the viewer (Python -> viewer sync).
	 */
	function applyModelToViewer(): void {
		if (!viewerReady || isDisposed) return;

		try {
			setContrast(viewerId, model.get("contrast"));
			setBias(viewerId, model.get("bias"));
			setStretchMode(viewerId, model.get("stretch_mode"));

			const xlim = model.get("xlim") as [number, number];
			const ylim = model.get("ylim") as [number, number];
			if (xlim[0] !== xlim[1] && ylim[0] !== ylim[1]) {
				setViewBounds(viewerId, xlim[0], xlim[1], ylim[0], ylim[1]);
			}
			// Apply zoom after bounds so explicit zoom is preserved.
			setZoom(viewerId, model.get("zoom"));

			// Rotation state
			setRotation(viewerId, model.get("rotation"));
			const pivot = model.get("pivot") as [number, number];
			setPivotPoint(viewerId, pivot[0], pivot[1]);
			setShowPivotMarker(viewerId, model.get("show_pivot_marker"));
			setOverlayMessage(viewerId, model.get("overlay_message"));
			setMarkers(viewerId, model.get("markers") as [number, number][]);
		} catch (e) {
			// Viewer may not be ready yet
		}
	}

	/**
	 * Apply initial viewer configuration passed from Python.
	 */
	function applyInitialViewerConfig(): void {
		if (!viewerReady || isDisposed) return;

		try {
			const config = model.get("viewer_config");
			setViewerState(viewerId, config);
		} catch (e) {
			// Viewer may not be ready yet
		}
	}

	/**
	 * Update the image data in the viewer.
	 */
	function updateImage(): void {
		if (!viewerReady || isDisposed) return;

		const imageUpdateToken = model.get("_image_update_token");
		if (imageUpdateToken === lastAppliedImageToken) return;
		const dataView = model.get("data");
		const imageWidth = model.get("image_width");
		const imageHeight = model.get("image_height");
		const dtype = model.get("dtype");

		if (dataView && dataView.byteLength > 0 && imageWidth > 0 && imageHeight > 0) {
			// Extract ArrayBuffer from DataView (slice creates a new ArrayBuffer, not SharedArrayBuffer)
			const buffer = dataView.buffer.slice(
				dataView.byteOffset,
				dataView.byteOffset + dataView.byteLength
			) as ArrayBuffer;
			// Tag the image with its slice indices so the widget can track the
			// slider position and correlate play-mode prefetch responses.
			const indices = model.get("current_slice_indices");
			setSliceData(viewerId, buffer, imageWidth, imageHeight, dtype, indices);
			lastAppliedImageToken = imageUpdateToken;
		}
	}

	/**
	 * Update the widget container dimensions.
	 */
	function updateDimensions(): void {
		if (isDisposed) return;
		container.style.width = `${model.get("widget_width")}px`;
		container.style.height = `${model.get("widget_height")}px`;
	}

	/**
	 * Declare the cube's sliceable leading axes to the viewer widget, which
	 * renders the slice + play controls itself. Slices are still served on
	 * demand from Python in response to onSliceRequest.
	 */
	function updateCube(): void {
		if (!viewerReady || isDisposed) return;
		const shape = model.get("shape");
		const leading = shape.length > 2 ? shape.slice(0, shape.length - 2) : [];
		setCube(viewerId, leading);
	}

	/**
	 * Handle a slice request from the viewer widget (slider drag or play loop).
	 * Updating the traitlet drives Python's on-demand slice extraction, which
	 * pushes the new slice back via updateImage -> setSliceData.
	 */
	function handleSliceRequest(indices: number[]): void {
		if (!viewerReady || isDisposed) return;
		const current = model.get("current_slice_indices") as number[];
		if (
			current.length === indices.length &&
			current.every((v, i) => v === indices[i])
		) {
			return; // already on this slice
		}
		model.set("current_slice_indices", indices);
		model.save_changes(); // trigger Python-side slice extraction
	}

	// Wait for the container to be in the DOM before initializing the viewer.
	// This is necessary because createViewer uses document.getElementById(),
	// which only works for elements attached to the document.
	waitForDOMConnection(container)
		.then(() => {
			if (isDisposed) return;
			return createViewer(viewerId);
		})
		.then(() => {
			if (isDisposed) return;
			viewerReady = true;
			// Register callback for state changes from the viewer
			onStateChange(viewerId, handleViewerStateChange);
			onClick(viewerId, handleShiftClick);
			onSliceRequest(viewerId, handleSliceRequest);
			// Declare cube axes (slice UI lives in the widget), then push the
			// initial slice.
			updateCube();
			updateImage();
			applyInitialViewerConfig();
			// Initial sync from viewer to get default values
			initialSyncViewerToModel();
		})
		.catch((err) => {
			if (!isDisposed) {
				console.error("Failed to create viewarr viewer:", err);
			}
		});

	// Listen for data changes from Python
	model.on("change:data", updateImage);
	model.on("change:image_width", updateImage);
	model.on("change:image_height", updateImage);
	model.on("change:dtype", updateImage);
	model.on("change:_image_update_token", updateImage);

	// Listen for widget dimension changes
	model.on("change:widget_width", updateDimensions);
	model.on("change:widget_height", updateDimensions);

	// Re-declare cube axes when the array shape changes (slice UI is in the widget)
	model.on("change:shape", updateCube);
	model.on("change:viewer_config", applyInitialViewerConfig);

	// Listen for viewer property changes from Python (only apply if not triggered by viewer sync)
	function handlePropertyChange(): void {
		// Skip direct echo while we're actively applying viewer -> model updates.
		if (updatingFromViewer) {
			return;
		}
		// Clear one-shot sync hint but still apply Python-driven updates (e.g. markers).
		if (model.get("_sync_from_viewer")) {
			model.set("_sync_from_viewer", false);
		}
		applyModelToViewer();
	}

	model.on("change:contrast", handlePropertyChange);
	model.on("change:bias", handlePropertyChange);
	model.on("change:stretch_mode", handlePropertyChange);
	model.on("change:zoom", handlePropertyChange);
	model.on("change:xlim", handlePropertyChange);
	model.on("change:ylim", handlePropertyChange);
	model.on("change:rotation", handlePropertyChange);
	model.on("change:pivot", handlePropertyChange);
	model.on("change:show_pivot_marker", handlePropertyChange);
	model.on("change:overlay_message", handlePropertyChange);
	model.on("change:markers", handlePropertyChange);

	// Cleanup when widget is removed
	return () => {
		isDisposed = true;
		if (viewerReady) {
			clearCallbacks(viewerId);
			destroyViewer(viewerId);
		}
	};
}

export default { render };
