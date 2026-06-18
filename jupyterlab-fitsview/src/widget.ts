import {
  DocumentModel,
  DocumentRegistry,
  DocumentWidget
} from '@jupyterlab/docregistry';
import { Widget } from '@lumino/widgets';
import { Message } from '@lumino/messaging';
import {
  requestAPI,
  requestBinaryAPI,
  requestBinaryAPIWithProgress,
  ArrayType,
  calculateSliceByteSize,
  formatByteSize
} from './handler';

// Maximum auto-fetch size in bytes (5 MB)
const MAX_AUTO_FETCH_SIZE = 5 * 1024 * 1024;

// Dynamically import viewarr for WASM viewer
// Using dynamic import allows graceful degradation if viewarr isn't built
let viewarr: typeof import('viewarr') | null = null;
const viewarrPromise = import('viewarr')
  .then(module => {
    viewarr = module;
  })
  .catch(err => {
    console.warn('viewarr module not available, image viewer disabled:', err);
  });

/**
 * FITS metadata response from the server
 */
export interface IFITSMetadata {
  path: string;
  hdus: IHDUInfo[];
}

/**
 * Information about a single HDU
 */
export interface IHDUInfo {
  index: number;
  name: string;
  type: string;
  header: string; // Raw 80-column FITS header string
  shape: number[] | null;
  arrayType: ArrayType | null;
}

/**
 * The FITS document model - uses base DocumentModel since we don't load content
 */
export class FITSModel extends DocumentModel {
  // Uses all defaults from DocumentModel
}

/**
 * State for slice navigation on a single HDU
 */
interface ISliceState {
  hduIndex: number;
  shape: number[];
  // Current index for each leading axis (all but last 2)
  sliceIndices: number[];
}

/**
 * The FITS viewer panel widget
 */
export class FITSPanel extends Widget {
  private static _instanceCounter = 0;
  private _baseViewerId: string;
  // Map of HDU index to viewer container element
  private _viewerContainers: Map<number, HTMLDivElement> = new Map();
  // Map of HDU index to slice state
  private _sliceStates: Map<number, ISliceState> = new Map();
  // Currently active HDU index
  private _activeHduIndex: number | null = null;
  private _sliceControlsContainer: HTMLDivElement | null = null;
  private _fetchAbortController: AbortController | null = null;

  constructor(context: DocumentRegistry.IContext<DocumentModel>) {
    super();
    this._context = context;
    this.addClass('jp-FITSViewer');

    // Generate unique base viewer ID (each HDU will get its own suffix)
    this._baseViewerId = `fitsview-${FITSPanel._instanceCounter++}`;

    // Create content container
    this._content = document.createElement('div');
    this._content.className = 'jp-FITSViewer-content';
    this.node.appendChild(this._content);

    // Load metadata when context is ready
    void context.ready.then(() => {
      void this._loadMetadata();
    });
  }

  /**
   * Get viewer ID for a specific HDU
   */
  private _getViewerId(hduIndex: number): string {
    return `${this._baseViewerId}-hdu${hduIndex}`;
  }

  /**
   * Load FITS metadata from the server
   */
  private async _loadMetadata(): Promise<void> {
    const path = this._context.path;
    this._content.innerHTML = `<p>Loading metadata for ${path}...</p>`;

    try {
      const metadata = await requestAPI<IFITSMetadata>(
        `metadata?path=${encodeURIComponent(path)}`
      );
      this._metadata = metadata;
      this._renderMetadata();
    } catch (error) {
      this._content.innerHTML = `<p class="jp-FITSViewer-error">Error loading FITS metadata: ${error}</p>`;
    }
  }

  /**
   * Render the metadata display
   */
  private _renderMetadata(): void {
    if (!this._metadata) {
      return;
    }

    const { hdus } = this._metadata;

    // Find all viewable HDUs (2D+ data with known array type)
    const viewableHdus = hdus.filter(
      hdu => hdu.shape && hdu.shape.length >= 2 && hdu.arrayType
    );

    // Create viewer containers HTML for each viewable HDU
    let viewerContainersHtml = '';
    for (const hdu of viewableHdus) {
      const viewerId = this._getViewerId(hdu.index);
      viewerContainersHtml += `
        <div id="${viewerId}" class="jp-FITSViewer-viewerContainer" data-hdu="${hdu.index}" style="display: none;">
          <div class="jp-FITSViewer-viewerPlaceholder">Loading...</div>
        </div>
      `;
    }

    // If no viewable HDUs, show a placeholder
    if (viewableHdus.length === 0) {
      viewerContainersHtml = `
        <div class="jp-FITSViewer-viewerContainer">
          <div class="jp-FITSViewer-viewerPlaceholder">No viewable image data</div>
        </div>
      `;
    }

    // Build HDU buttons for horizontal bar (all HDUs, viewable or not)
    let hduButtonsHtml = '';
    for (const hdu of hdus) {
      const isViewable = hdu.shape && hdu.shape.length >= 2 && hdu.arrayType;
      const hduName = hdu.name || `HDU ${hdu.index}`;

      // Build info line: shape × type, row count, or dash
      let infoStr: string;
      if (hdu.shape && hdu.shape.length >= 2 && hdu.arrayType) {
        // Image data: show dimensions and type
        infoStr = `${hdu.shape.join('×')} [${hdu.arrayType}]`;
      } else if (hdu.shape && hdu.shape.length === 1) {
        // Table data: show row count
        infoStr = `${hdu.shape[0]} rows`;
      } else {
        // No data
        infoStr = '—';
      }

      const buttonClass = isViewable
        ? 'jp-FITSViewer-hduButton'
        : 'jp-FITSViewer-hduButton jp-FITSViewer-hduButton-disabled';
      const disabledAttr = isViewable ? '' : 'disabled';

      hduButtonsHtml += `
        <button class="${buttonClass}" data-hdu="${hdu.index}" ${disabledAttr}>
          <div class="jp-FITSViewer-hduContent">
            <span class="jp-FITSViewer-hduName">${hdu.index}: ${hduName}</span>
            <span class="jp-FITSViewer-hduInfo">${infoStr}</span>
          </div>
          <a href="#" class="jp-FITSViewer-headerIcon" data-hdu="${hdu.index}" title="View Header">ⓘ</a>
        </button>
      `;
    }

    // Create main layout with HDU bar, slice controls, and viewer (no sidebar)
    const html = `
      <div class="jp-FITSViewer-layout">
        <div class="jp-FITSViewer-viewerPanel">
          <div class="jp-FITSViewer-controlBar">
            <div class="jp-FITSViewer-hduBar">${hduButtonsHtml}</div>
            <div id="${this._baseViewerId}-controls" class="jp-FITSViewer-sliceControls"></div>
          </div>
          <div class="jp-FITSViewer-viewerStack">
            ${viewerContainersHtml}
          </div>
        </div>
      </div>
    `;

    this._content.innerHTML = html;

    // Store references to viewer containers for each viewable HDU
    this._viewerContainers.clear();
    for (const hdu of viewableHdus) {
      const viewerId = this._getViewerId(hdu.index);
      const container = document.getElementById(viewerId) as HTMLDivElement;
      if (container) {
        this._viewerContainers.set(hdu.index, container);
      }
    }

    this._sliceControlsContainer = document.getElementById(
      `${this._baseViewerId}-controls`
    ) as HTMLDivElement;

    // Wait for viewarr to be ready, then auto-display first viewable HDU
    // Note: We don't create the viewer here - it's created on-demand when needed
    void viewarrPromise.then(() => {
      this._autoDisplayFirstHDU();
    });

    // Attach event listeners to HDU buttons for selection
    const hduButtons = this._content.querySelectorAll(
      '.jp-FITSViewer-hduButton'
    );
    hduButtons.forEach(btn => {
      btn.addEventListener('click', e => {
        // Don't trigger if clicking on the header icon
        if (
          (e.target as HTMLElement).classList.contains(
            'jp-FITSViewer-headerIcon'
          )
        ) {
          return;
        }
        const hduIndex = parseInt((btn as HTMLElement).dataset.hdu || '0', 10);
        void this._switchToHDU(hduIndex);
      });
    });

    // Attach event listeners to header icons
    const headerIcons = this._content.querySelectorAll(
      '.jp-FITSViewer-headerIcon'
    );
    headerIcons.forEach(icon => {
      icon.addEventListener('click', e => {
        e.preventDefault();
        e.stopPropagation();
        const hduIndex = parseInt((icon as HTMLElement).dataset.hdu || '0', 10);
        this._openHeaderWindow(hduIndex);
      });
    });
  }

  /**
   * Switch to displaying a different HDU
   */
  private async _switchToHDU(hduIndex: number): Promise<void> {
    if (this._activeHduIndex === hduIndex) {
      return; // Already showing this HDU
    }

    // Hide the current viewer container
    if (this._activeHduIndex !== null) {
      const currentContainer = this._viewerContainers.get(this._activeHduIndex);
      if (currentContainer) {
        currentContainer.style.display = 'none';
      }
      // Remove active class from current HDU button
      const currentHduBtn = this._content.querySelector(
        `.jp-FITSViewer-hduButton[data-hdu="${this._activeHduIndex}"]`
      );
      currentHduBtn?.classList.remove('jp-FITSViewer-hduButton-active');
    }

    // Show the new viewer container
    const newContainer = this._viewerContainers.get(hduIndex);
    if (newContainer) {
      newContainer.style.display = 'flex';
    }

    // Add active class to new HDU button
    const newHduBtn = this._content.querySelector(
      `.jp-FITSViewer-hduButton[data-hdu="${hduIndex}"]`
    );
    newHduBtn?.classList.add('jp-FITSViewer-hduButton-active');

    this._activeHduIndex = hduIndex;

    // Load the HDU image (will create viewer if needed, or use existing)
    await this._viewHDUImage(hduIndex);
  }

  /**
   * Open HDU header in a new window
   */
  private _openHeaderWindow(hduIndex: number): void {
    const hdu = this._metadata?.hdus.find(h => h.index === hduIndex);
    if (!hdu || !hdu.header) {
      return;
    }

    const fileName = this._context.path.split('/').pop() || 'file';
    const hduName = hdu.name || `HDU ${hduIndex}`;
    const title = `${fileName} - ${hduName} Header`;

    // Create a new window with the header content
    const headerWindow = window.open('', '_blank', 'width=800,height=600');
    if (headerWindow) {
      headerWindow.document.write(`<!DOCTYPE html>
<html>
<head>
  <title>${title}</title>
  <style>
    body {
      font-family: monospace;
      padding: 16px;
      margin: 0;
      background: #1e1e1e;
      color: #d4d4d4;
    }
    pre {
      white-space: pre;
      margin: 0;
      line-height: 1.4;
    }
  </style>
</head>
<body>
<pre>${hdu.header.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')}</pre>
</body>
</html>`);
      headerWindow.document.close();
    }
  }

  /**
   * Ensure the viewarr viewer exists for a specific HDU, creating it if necessary.
   * Call this before any operation that needs the viewer.
   */
  private async _ensureViewer(hduIndex: number): Promise<boolean> {
    await viewarrPromise;

    const viewerContainer = this._viewerContainers.get(hduIndex);
    const viewerId = this._getViewerId(hduIndex);

    if (!viewarr || !viewerContainer) {
      return false;
    }

    // If viewer already exists, we're good
    if (viewarr.hasViewer(viewerId)) {
      return true;
    }

    try {
      // Clear any placeholder content
      viewerContainer.innerHTML = '';
      await viewarr.createViewer(viewerId);
      return true;
    } catch (error) {
      console.error('Failed to create viewarr viewer:', error);
      return false;
    }
  }

  /**
   * Automatically display the first HDU with viewable 2D+ data
   */
  private _autoDisplayFirstHDU(): void {
    if (!this._metadata || !viewarr) {
      return;
    }

    // Find first HDU with 2D+ data
    const viewableHDU = this._metadata.hdus.find(
      hdu => hdu.shape && hdu.shape.length >= 2 && hdu.arrayType
    );

    if (viewableHDU) {
      void this._switchToHDU(viewableHDU.index);
    }
  }

  /**
   * View full HDU image in the viewer
   */
  private async _viewHDUImage(hduIndex: number): Promise<void> {
    if (!viewarr) {
      console.warn('viewarr not available');
      return;
    }

    const hdu = this._metadata?.hdus.find(h => h.index === hduIndex);
    if (!hdu || !hdu.shape || hdu.shape.length < 2 || !hdu.arrayType) {
      return;
    }

    const shape = hdu.shape;
    const numLeadingAxes = shape.length - 2;

    // Get or create slice state for this HDU
    let sliceState = this._sliceStates.get(hduIndex);
    if (!sliceState) {
      sliceState = {
        hduIndex,
        shape,
        sliceIndices: new Array(numLeadingAxes).fill(0)
      };
      this._sliceStates.set(hduIndex, sliceState);
    }

    // Render slice controls for this HDU
    this._renderSliceControls();

    // Check if viewer already has data loaded (don't re-fetch if switching back)
    const viewerId = this._getViewerId(hduIndex);
    if (viewarr.hasViewer(viewerId)) {
      // Viewer exists and has data, no need to fetch
      return;
    }

    // Calculate the byte size for the 2D slice
    const sliceByteSize = calculateSliceByteSize(shape, hdu.arrayType);

    if (sliceByteSize <= MAX_AUTO_FETCH_SIZE) {
      // Small enough to auto-fetch
      await this._fetchAndDisplaySlice();
    } else {
      // Too large - show fetch prompt
      this._showLargeImagePrompt(sliceByteSize);
    }
  }

  /**
   * Get the current slice state for the active HDU
   */
  private _getActiveSliceState(): ISliceState | undefined {
    if (this._activeHduIndex === null) {
      return undefined;
    }
    return this._sliceStates.get(this._activeHduIndex);
  }

  /**
   * Get the viewer container for the active HDU
   */
  private _getActiveViewerContainer(): HTMLDivElement | undefined {
    if (this._activeHduIndex === null) {
      return undefined;
    }
    return this._viewerContainers.get(this._activeHduIndex);
  }

  /**
   * Show a prompt for large images with fetch button and progress UI
   */
  private _showLargeImagePrompt(byteSize: number): void {
    const viewerContainer = this._getActiveViewerContainer();
    if (!viewerContainer) {
      return;
    }

    const sizeStr = formatByteSize(byteSize);
    viewerContainer.innerHTML = `
      <div class="jp-FITSViewer-largeImagePrompt">
        <p>This image slice is <strong>${sizeStr}</strong>, which exceeds the auto-fetch limit.</p>
        <button class="jp-FITSViewer-fetchButton jp-mod-styled">
          Fetch and Display
        </button>
        <div class="jp-FITSViewer-progressContainer" style="display: none;">
          <div class="jp-FITSViewer-progressBar">
            <div class="jp-FITSViewer-progressFill"></div>
          </div>
          <span class="jp-FITSViewer-progressText">0%</span>
          <button class="jp-FITSViewer-cancelButton jp-mod-styled jp-mod-warn">
            Cancel
          </button>
        </div>
      </div>
    `;

    const fetchButton = viewerContainer.querySelector(
      '.jp-FITSViewer-fetchButton'
    ) as HTMLButtonElement;
    const progressContainer = viewerContainer.querySelector(
      '.jp-FITSViewer-progressContainer'
    ) as HTMLDivElement;
    const progressFill = viewerContainer.querySelector(
      '.jp-FITSViewer-progressFill'
    ) as HTMLDivElement;
    const progressText = viewerContainer.querySelector(
      '.jp-FITSViewer-progressText'
    ) as HTMLSpanElement;
    const cancelButton = viewerContainer.querySelector(
      '.jp-FITSViewer-cancelButton'
    ) as HTMLButtonElement;

    fetchButton.addEventListener('click', () => {
      fetchButton.style.display = 'none';
      progressContainer.style.display = 'flex';
      void this._fetchAndDisplaySliceWithProgress(
        progressFill,
        progressText,
        progressContainer
      );
    });

    cancelButton.addEventListener('click', () => {
      if (this._fetchAbortController) {
        this._fetchAbortController.abort();
        this._fetchAbortController = null;
      }
      // Reset UI
      fetchButton.style.display = 'block';
      progressContainer.style.display = 'none';
      progressFill.style.width = '0%';
      progressText.textContent = '0%';
    });
  }

  /**
   * Fetch and display slice with progress tracking
   */
  private async _fetchAndDisplaySliceWithProgress(
    progressFill: HTMLDivElement,
    progressText: HTMLSpanElement,
    progressContainer: HTMLDivElement
  ): Promise<void> {
    const sliceState = this._getActiveSliceState();
    const viewerContainer = this._getActiveViewerContainer();
    const hduIndex = this._activeHduIndex;

    console.log('[FITSViewer] _fetchAndDisplaySliceWithProgress called');
    console.log('[FITSViewer] sliceState:', sliceState);
    console.log('[FITSViewer] viewarr:', viewarr);

    if (!sliceState || !viewarr || hduIndex === null) {
      console.warn('[FITSViewer] Early return: missing sliceState or viewarr');
      return;
    }

    const viewerId = this._getViewerId(hduIndex);
    const { shape, sliceIndices } = sliceState;

    // Build slice string
    const slices = shape
      .map((size, i) => {
        if (i < shape.length - 2) {
          const idx = sliceIndices[i];
          return `${idx}:${idx + 1}`;
        }
        return `0:${size}`;
      })
      .join(',');

    // Create abort controller
    this._fetchAbortController = new AbortController();

    try {
      const path = this._context.path;
      console.log('[FITSViewer] Fetching slice:', { path, hduIndex, slices });

      const {
        buffer,
        shape: resultShape,
        arrayType
      } = await requestBinaryAPIWithProgress(
        `slice?path=${encodeURIComponent(path)}&hdu=${hduIndex}&slices=${encodeURIComponent(slices)}`,
        (loaded, total) => {
          const percent = Math.round((loaded / total) * 100);
          progressFill.style.width = `${percent}%`;
          progressText.textContent = `${percent}% (${formatByteSize(loaded)} / ${formatByteSize(total)})`;
        },
        this._fetchAbortController.signal
      );

      console.log('[FITSViewer] Fetch complete:', {
        bufferLength: buffer.byteLength,
        resultShape,
        arrayType
      });
      this._fetchAbortController = null;

      // Get image dimensions
      const height = resultShape[resultShape.length - 2] || 1;
      const width = resultShape[resultShape.length - 1] || 1;
      console.log('[FITSViewer] Image dimensions:', { width, height });

      // Clear the progress UI and create the viewer
      // No viewer exists yet since we showed the prompt instead of creating one
      if (viewerContainer) {
        console.log('[FITSViewer] Clearing container and creating viewer');
        viewerContainer.innerHTML = '';
        await viewarr.createViewer(viewerId);
        console.log(
          '[FITSViewer] Viewer created, hasViewer:',
          viewarr.hasViewer(viewerId)
        );
      } else {
        console.warn('[FITSViewer] No viewer container!');
      }

      console.log('[FITSViewer] Calling setImageData');
      viewarr.setImageData(viewerId, buffer, width, height, arrayType);
      console.log('[FITSViewer] setImageData complete');
    } catch (error) {
      this._fetchAbortController = null;
      if ((error as Error).name === 'AbortError') {
        console.log('Fetch cancelled by user');
        return;
      }
      console.error('Failed to load slice:', error);
      progressContainer.innerHTML = `<span class="jp-FITSViewer-error">Error: ${error}</span>`;
    }
  }

  /**
   * Render slice navigation controls for leading axes
   */
  private _renderSliceControls(): void {
    const sliceState = this._getActiveSliceState();
    if (!this._sliceControlsContainer || !sliceState) {
      return;
    }

    const { shape, sliceIndices } = sliceState;
    const numLeadingAxes = shape.length - 2;

    if (numLeadingAxes === 0) {
      // No leading axes, hide controls
      this._sliceControlsContainer.innerHTML = '';
      return;
    }

    let html = '';
    for (let axis = 0; axis < numLeadingAxes; axis++) {
      const axisSize = shape[axis];
      const currentIndex = sliceIndices[axis];
      const axisLabel = numLeadingAxes === 1 ? 'Plane' : `Axis ${axis}`;

      html += `
        <div class="jp-FITSViewer-sliceControl" data-axis="${axis}">
          <button class="jp-FITSViewer-sliceButton jp-FITSViewer-prevButton"
                  data-axis="${axis}"
                  data-direction="prev"
                  ${currentIndex === 0 ? 'disabled' : ''}>
            ◀
          </button>
          <span class="jp-FITSViewer-sliceLabel">
            ${axisLabel}: <strong>${currentIndex + 1}</strong> / ${axisSize}
          </span>
          <button class="jp-FITSViewer-sliceButton jp-FITSViewer-nextButton"
                  data-axis="${axis}"
                  data-direction="next"
                  ${currentIndex >= axisSize - 1 ? 'disabled' : ''}>
            ▶
          </button>
        </div>
      `;
    }

    this._sliceControlsContainer.innerHTML = html;

    // Attach event listeners
    const buttons = this._sliceControlsContainer.querySelectorAll(
      '.jp-FITSViewer-sliceButton'
    );
    buttons.forEach(btn => {
      btn.addEventListener('click', e => {
        const target = e.currentTarget as HTMLElement;
        const axis = parseInt(target.dataset.axis || '0', 10);
        const direction = target.dataset.direction;
        this._navigateSlice(axis, direction === 'next' ? 1 : -1);
      });
    });
  }

  /**
   * Navigate to a different slice along a given axis
   */
  private _navigateSlice(axis: number, delta: number): void {
    const sliceState = this._getActiveSliceState();
    if (!sliceState) {
      return;
    }

    const { shape, sliceIndices } = sliceState;
    const axisSize = shape[axis];
    const newIndex = Math.max(
      0,
      Math.min(axisSize - 1, sliceIndices[axis] + delta)
    );

    if (newIndex !== sliceIndices[axis]) {
      sliceState.sliceIndices[axis] = newIndex;
      this._renderSliceControls();
      void this._fetchAndDisplaySlice();
    }
  }

  /**
   * Fetch and display the current slice based on slice state
   */
  private async _fetchAndDisplaySlice(): Promise<void> {
    const sliceState = this._getActiveSliceState();
    const hduIndex = this._activeHduIndex;

    if (!sliceState || !viewarr || hduIndex === null) {
      return;
    }

    // Ensure viewer exists (creates on-demand)
    const viewerReady = await this._ensureViewer(hduIndex);
    if (!viewerReady) {
      return;
    }

    const viewerId = this._getViewerId(hduIndex);
    const { shape, sliceIndices } = sliceState;

    // Build slice string: for leading axes use current index, for image axes use full extent
    const slices = shape
      .map((size, i) => {
        if (i < shape.length - 2) {
          const idx = sliceIndices[i];
          return `${idx}:${idx + 1}`;
        }
        return `0:${size}`;
      })
      .join(',');

    try {
      const path = this._context.path;
      const {
        buffer,
        shape: resultShape,
        arrayType
      } = await requestBinaryAPI(
        `slice?path=${encodeURIComponent(path)}&hdu=${hduIndex}&slices=${encodeURIComponent(slices)}`
      );

      // Get image dimensions (last two elements of shape)
      const height = resultShape[resultShape.length - 2] || 1;
      const width = resultShape[resultShape.length - 1] || 1;

      // Use arrayType from response for proper data interpretation
      viewarr.setImageData(viewerId, buffer, width, height, arrayType);
    } catch (error) {
      console.error('Failed to load slice:', error);
    }
  }

  /**
   * Handle dispose
   */
  protected onCloseRequest(msg: Message): void {
    // Clean up all viewarr instances
    if (viewarr) {
      this._viewerContainers.forEach((_, hduIndex) => {
        const viewerId = this._getViewerId(hduIndex);
        if (viewarr!.hasViewer(viewerId)) {
          viewarr!.destroyViewer(viewerId);
        }
      });
    }
    super.onCloseRequest(msg);
    this.dispose();
  }

  private _context: DocumentRegistry.IContext<DocumentModel>;
  private _content: HTMLDivElement;
  private _metadata: IFITSMetadata | null = null;
}

/**
 * A document widget for FITS files
 */
export class FITSDocument extends DocumentWidget<FITSPanel, DocumentModel> {
  constructor(options: DocumentWidget.IOptions<FITSPanel, DocumentModel>) {
    super(options);
  }
}
