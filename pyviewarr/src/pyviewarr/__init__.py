import importlib.metadata
import pathlib
from IPython.display import display
from dataclasses import asdict, dataclass
from typing import Any, Callable, Dict, Literal, Optional, Tuple, TYPE_CHECKING, Union

import anywidget
import numpy as np
import traitlets

if TYPE_CHECKING:
    from matplotlib.axes import Axes

try:
    __version__ = importlib.metadata.version("pyviewarr")
except importlib.metadata.PackageNotFoundError:
    __version__ = "unknown"

__all__ = (
    "ViewArrWidget",
    "ViewerConfig",
    "show",
    "ViewarrNormalize",
)

# Mapping from numpy dtype to viewarr type string
_DTYPE_MAP = {
    np.dtype("int8"): "i8",
    np.dtype("uint8"): "u8",
    np.dtype("int16"): "i16",
    np.dtype("uint16"): "u16",
    np.dtype("int32"): "i32",
    np.dtype("uint32"): "u32",
    np.dtype("int64"): "i64",
    np.dtype("uint64"): "u64",
    np.dtype("float32"): "f32",
    np.dtype("float64"): "f64",
}

_CMAP_CANONICAL_NAMES = {
    "gray": "gray",
    "grayscale": "gray",
    "greyscale": "gray",
    "inferno": "inferno",
    "magma": "magma",
    "rdbu": "RdBu",
    "rdylbu": "RdYlBu",
}

ShiftClickCallback = Callable[[float, float], None]
MarkerPoint = Tuple[float, float]


def _numpy_dtype_to_viewarr(dtype: np.dtype) -> str:
    """Convert numpy dtype to viewarr type string."""
    if dtype in _DTYPE_MAP:
        return _DTYPE_MAP[dtype]
    raise ValueError(
        f"Unsupported dtype: {dtype}. Supported: {list(_DTYPE_MAP.keys())}"
    )


# =========================================================================
# ViewarrNormalize: Matplotlib-compatible normalization class
# =========================================================================

# Try to import matplotlib's Normalize base class, but don't require it
try:
    from matplotlib.colors import Normalize as _NormalizeBase

    _HAS_MATPLOTLIB = True
except ImportError:
    _HAS_MATPLOTLIB = False
    _NormalizeBase = object  # Fall back to plain object


class ViewarrNormalize(_NormalizeBase):
    """Matplotlib-compatible normalization with DS9-style contrast/bias and optional log stretch.

    This class replicates the normalization pipeline used by the viewarr WASM viewer,
    allowing you to render images with matplotlib using the same stretch settings
    you've dialed in interactively.

    The normalization pipeline is:
    1. Normalize to 0-1 range based on vmin/vmax (or symmetric around zero)
    2. Optionally apply log stretch: log10(1000*x + 1) / log10(1000)
    3. Apply DS9 contrast/bias: (x - bias) * contrast + 0.5

    Parameters
    ----------
    vmin : float, optional
        Minimum data value. If None, will be set from data.
    vmax : float, optional
        Maximum data value. If None, will be set from data.
    contrast : float, default=1.0
        Contrast value (0.0 to 10.0). Higher values increase contrast.
    bias : float, default=0.5
        Bias value (0.0 to 1.0). Controls the midpoint of the stretch.
    log : bool, default=False
        If True, apply flexible log stretch before contrast/bias.
    symmetric : bool, default=False
        If True, scale symmetrically around zero (for diverging colormaps).
        In symmetric mode, bias is locked to 0.5.
    clip : bool, default=True
        If True, clip output to [0, 1] range.

    Examples
    --------
    >>> norm = ViewarrNormalize(vmin=0, vmax=100, contrast=2.0, bias=0.4)
    >>> ax.imshow(data, norm=norm, cmap='gray')

    >>> # Using log stretch
    >>> norm = ViewarrNormalize(vmin=1, vmax=1000, log=True)
    >>> ax.imshow(data, norm=norm, cmap='inferno')

    >>> # Symmetric mode for diverging data
    >>> norm = ViewarrNormalize(symmetric=True, contrast=1.5)
    >>> ax.imshow(data, norm=norm, cmap='RdBu_r')
    """

    # Log stretch exponent (DS9 default for optical images)
    LOG_EXPONENT = 1000.0

    def __init__(
        self,
        vmin: Optional[float] = None,
        vmax: Optional[float] = None,
        contrast: float = 1.0,
        bias: float = 0.5,
        log: bool = False,
        symmetric: bool = False,
        clip: bool = True,
    ):
        # Initialize matplotlib base class if available
        if _HAS_MATPLOTLIB:
            super().__init__(vmin=vmin, vmax=vmax, clip=clip)
        else:
            self.vmin = vmin
            self.vmax = vmax
            self.clip = clip

        self.contrast = contrast
        self.bias = bias
        self.log = log
        self.symmetric = symmetric

    def __call__(self, value, clip=None):
        """Normalize value(s) to 0-1 range.

        Parameters
        ----------
        value : array-like
            Data to normalize.
        clip : bool, optional
            Override instance clip setting.

        Returns
        -------
        np.ma.MaskedArray
            Normalized values in [0, 1] range.
        """
        if clip is None:
            clip = self.clip

        # Convert to masked array to handle NaN/Inf
        result = np.ma.asarray(value)

        # Determine vmin/vmax if not set
        vmin = self.vmin
        vmax = self.vmax
        if vmin is None:
            vmin = float(np.nanmin(result))
        if vmax is None:
            vmax = float(np.nanmax(result))

        # Step 1: Determine scaling range
        if self.symmetric:
            abs_max = max(abs(vmin), abs(vmax))
            scale_min, scale_max = -abs_max, abs_max
        else:
            scale_min, scale_max = vmin, vmax

        # Step 2: Normalize to 0-1
        range_val = scale_max - scale_min
        if abs(range_val) < 1e-15:
            normalized = np.zeros_like(result)
        else:
            normalized = (result - scale_min) / range_val

        if clip:
            normalized = np.clip(normalized, 0, 1)

        # Step 3: Apply log stretch if enabled
        if self.log:
            stretched = np.log10(self.LOG_EXPONENT * normalized + 1) / np.log10(
                self.LOG_EXPONENT
            )
        else:
            stretched = normalized

        # Step 4: Apply contrast/bias (DS9 formula)
        # In symmetric mode, bias is locked to 0.5 to keep zero at center
        bias = 0.5 if self.symmetric else self.bias
        output = (stretched - bias) * self.contrast + 0.5

        if clip:
            output = np.clip(output, 0, 1)

        return np.ma.array(output, mask=np.ma.getmask(result))

    def inverse(self, value):
        """Inverse transform (not fully implemented for log stretch)."""
        # This is a simplified inverse that ignores log stretch
        bias = 0.5 if self.symmetric else self.bias
        x = (value - 0.5) / self.contrast + bias

        vmin = self.vmin if self.vmin is not None else 0
        vmax = self.vmax if self.vmax is not None else 1

        if self.symmetric:
            abs_max = max(abs(vmin), abs(vmax))
            scale_min, scale_max = -abs_max, abs_max
        else:
            scale_min, scale_max = vmin, vmax

        return x * (scale_max - scale_min) + scale_min

    def autoscale(self, A):
        """Set vmin/vmax from data."""
        self.vmin = float(np.nanmin(A))
        self.vmax = float(np.nanmax(A))

    def autoscale_None(self, A):
        """Set vmin/vmax from data only if not already set."""
        if self.vmin is None:
            self.vmin = float(np.nanmin(A))
        if self.vmax is None:
            self.vmax = float(np.nanmax(A))

    def scaled(self):
        """Return whether vmin and vmax are set."""
        return self.vmin is not None and self.vmax is not None


@dataclass(kw_only=True)
class ViewerConfig:
    """Initial viewer state configuration.

    Field names use Python style and are converted to JavaScript state keys.
    Any field left as None is omitted from the JS config object.
    Use ``stretch`` and ``cmap`` for stretch mode and colormap, respectively.
    ``cmap`` is case-insensitive for supported colormap names.
    """

    contrast: Optional[float] = None
    bias: Optional[float] = None
    stretch: Optional[Literal["linear", "log", "symmetric"]] = None
    zoom: Optional[float] = None
    cmap: Optional[str] = None
    colormap_reversed: Optional[bool] = None
    vmin: Optional[float] = None
    vmax: Optional[float] = None
    xlim: Optional[Tuple[float, float]] = None
    ylim: Optional[Tuple[float, float]] = None
    rotation: Optional[float] = None
    pivot: Optional[Tuple[float, float]] = None
    show_pivot_marker: Optional[bool] = None
    markers: Optional[list[MarkerPoint]] = None
    on_shift_click: Optional[ShiftClickCallback] = None
    overlay_message: Optional[str] = None

    def to_js_dict(self) -> Dict[str, Any]:
        """Convert config to the JS object shape expected by the frontend."""
        raw = asdict(self)
        raw.pop("on_shift_click", None)
        raw.pop("overlay_message", None)
        if raw.get("markers") is not None:
            raw["markers"] = [
                (float(point[0]), float(point[1])) for point in raw["markers"]
            ]
        if raw.get("cmap") is not None:
            cmap = str(raw["cmap"]).strip()
            raw["cmap"] = _CMAP_CANONICAL_NAMES.get(cmap.lower(), cmap)
        key_map = {
            "stretch": "stretchMode",
            "cmap": "colormap",
            "colormap_reversed": "colormapReversed",
            "show_pivot_marker": "showPivotMarker",
        }
        return {
            key_map.get(key, key): value
            for key, value in raw.items()
            if value is not None
        }


# Colormap name mapping from viewarr to matplotlib
_COLORMAP_MAP = {
    "gray": "gray",
    "inferno": "inferno",
    "magma": "magma",
    "RdBu": "RdBu_r",  # Reversed to match matplotlib convention
    "RdYlBu": "RdYlBu_r",
}


class ViewArrWidget(anywidget.AnyWidget):
    """Anywidget for displaying 2D arrays using the viewarr WASM viewer."""

    _esm = pathlib.Path(__file__).parent / "static" / "widget.js"
    _css = pathlib.Path(__file__).parent / "static" / "widget.css"

    # Binary image data (synced as DataView in JavaScript)
    data = traitlets.Bytes(b"").tag(sync=True)

    # Image dimensions
    image_width = traitlets.Int(0).tag(sync=True)
    image_height = traitlets.Int(0).tag(sync=True)

    # Monotonic token that increments whenever image payload traits are updated.
    # Frontend applies setImageData when it observes a new token value.
    _image_update_token = traitlets.Int(0).tag(sync=True)

    # Data type string for viewarr (e.g., "f32", "u16")
    dtype = traitlets.Unicode("f64").tag(sync=True)

    # Widget display dimensions (CSS pixels)
    widget_width = traitlets.Int(800).tag(sync=True)
    widget_height = traitlets.Int(600).tag(sync=True)

    # Array shape (list of dimensions)
    shape = traitlets.List(traitlets.Int()).tag(sync=True)

    # Current slice indices for leading axes (empty for 2D arrays)
    current_slice_indices = traitlets.List(traitlets.Int()).tag(sync=True)

    # Optional initial viewer state object (mapped to JS state keys)
    viewer_config = traitlets.Dict(default_value={}).tag(sync=True)
    # Optional overlay message shown at bottom-center of the viewer
    overlay_message = traitlets.Unicode("").tag(sync=True)
    # Latest shift-click event from frontend: {"x": float, "y": float, "token": int}
    _shift_click_event = traitlets.Dict(default_value={}).tag(sync=True)

    # =========================================================================
    # Viewer state properties (bidirectional sync with frontend)
    # =========================================================================

    # Contrast value (0.0 to 10.0, default 1.0)
    contrast = traitlets.Float(1.0).tag(sync=True)

    # Bias value (0.0 to 1.0, default 0.5)
    bias = traitlets.Float(0.5).tag(sync=True)

    # Stretch mode: "linear", "log", or "symmetric"
    stretch_mode = traitlets.Unicode("linear").tag(sync=True)

    # Zoom level (1.0 = fit to view)
    zoom = traitlets.Float(1.0).tag(sync=True)

    # Viewport bounds in pixel coordinates
    xlim = traitlets.Tuple(
        traitlets.Float(), traitlets.Float(), default_value=(0.0, 0.0)
    ).tag(sync=True)
    ylim = traitlets.Tuple(
        traitlets.Float(), traitlets.Float(), default_value=(0.0, 0.0)
    ).tag(sync=True)

    # Colormap name (read from viewer)
    colormap = traitlets.Unicode("gray").tag(sync=True)

    # Whether colormap is reversed (read from viewer)
    colormap_reversed = traitlets.Bool(False).tag(sync=True)

    # Data value range (read from viewer after image load)
    vmin = traitlets.Float(0.0).tag(sync=True)
    vmax = traitlets.Float(1.0).tag(sync=True)

    # =========================================================================
    # Rotation state properties (bidirectional sync with frontend)
    # =========================================================================

    # Rotation angle in degrees (counter-clockwise, math convention)
    rotation = traitlets.Float(0.0).tag(sync=True)

    # Pivot point in image coordinates (default is center)
    pivot = traitlets.Tuple(
        traitlets.Float(), traitlets.Float(), default_value=(0.0, 0.0)
    ).tag(sync=True)

    # Whether to show the pivot marker
    show_pivot_marker = traitlets.Bool(False).tag(sync=True)
    # Marker list as continuous image coordinates: [(x, y), ...]
    markers = traitlets.List(
        traitlets.Tuple(traitlets.Float(), traitlets.Float()), default_value=[]
    ).tag(sync=True)

    # Internal flag to prevent feedback loops during sync
    _sync_from_viewer = traitlets.Bool(False).tag(sync=True)

    def __init__(
        self,
        viewer_config: Optional[Union[ViewerConfig, Dict[str, Any]]] = None,
        **kwargs,
    ):
        shift_click_callback = None
        if viewer_config is not None:
            if isinstance(viewer_config, ViewerConfig):
                shift_click_callback = viewer_config.on_shift_click
                if viewer_config.overlay_message is not None:
                    kwargs["overlay_message"] = viewer_config.overlay_message
                if viewer_config.markers is not None:
                    kwargs["markers"] = [
                        (float(x), float(y)) for (x, y) in viewer_config.markers
                    ]
                kwargs["viewer_config"] = viewer_config.to_js_dict()
            else:
                config_dict = dict(viewer_config)
                shift_click_callback = config_dict.pop("on_shift_click", None)
                overlay_message = config_dict.pop("overlay_message", None)
                markers = config_dict.get("markers")
                if overlay_message is not None:
                    kwargs["overlay_message"] = overlay_message
                if markers is not None:
                    kwargs["markers"] = [(float(x), float(y)) for (x, y) in markers]
                kwargs["viewer_config"] = ViewerConfig(**config_dict).to_js_dict()
        super().__init__(**kwargs)
        self._on_shift_click = shift_click_callback
        self._array = None
        self.observe(self._on_slice_indices_changed, names=["current_slice_indices"])
        self.observe(self._on_shift_click_event, names=["_shift_click_event"])

    def set_shift_click_callback(
        self, callback: Optional[ShiftClickCallback]
    ) -> None:
        """Set or clear the Python callback invoked on shift-click events."""
        self._on_shift_click = callback

    def _on_shift_click_event(self, change):
        callback = self._on_shift_click
        if callback is None:
            return
        event = change.get("new", {})
        if not isinstance(event, dict):
            return
        x = event.get("x")
        y = event.get("y")
        if isinstance(x, (int, float)) and isinstance(y, (int, float)):
            callback(float(x), float(y))

    def _on_slice_indices_changed(self, change):
        """Update the displayed slice when slice indices change."""
        self._update_slice()

    def _update_slice(self):
        """Compute the current 2D slice and update traits."""
        if self._array is None:
            return

        arr = self._array
        indices = self.current_slice_indices

        # Slice the array: leading axes use indices, last two are full slices
        if len(indices) > 0:
            slice_obj = tuple(indices) + (slice(None), slice(None))
            slice_arr = arr[slice_obj]
        else:
            slice_arr = arr

        # Ensure slice is contiguous and little-endian
        slice_arr = np.ascontiguousarray(slice_arr)
        slice_arr = slice_arr.astype(slice_arr.dtype.newbyteorder("<"))

        self.dtype = _numpy_dtype_to_viewarr(slice_arr.dtype)
        self.image_height, self.image_width = slice_arr.shape
        self.data = slice_arr.tobytes()
        self._image_update_token += 1

    def set_array(self, arr: np.ndarray) -> None:
        """Set the array data to display.

        Args:
            arr: A numpy array to display. Last two axes are treated as (y, x).
                 Leading axes can be navigated with slice controls.
        """
        if arr.ndim < 2:
            raise ValueError(
                f"Expected array with at least 2 dimensions, got {arr.ndim}D"
            )

        # Store the full array
        self._array = arr

        # Set shape
        self.shape = list(arr.shape)

        # Initialize slice indices for leading axes
        num_leading_axes = arr.ndim - 2
        self.current_slice_indices = [0] * num_leading_axes

        # Update the displayed slice
        self._update_slice()

    def get_current_slice(self) -> np.ndarray:
        """Get the current 2D slice being displayed.

        Returns
        -------
        np.ndarray
            The 2D array currently being displayed.
        """
        if self._array is None:
            raise ValueError("No array has been set")

        indices = self.current_slice_indices
        if len(indices) > 0:
            slice_obj = tuple(indices) + (slice(None), slice(None))
            return self._array[slice_obj]
        return self._array

    def get_normalization(self) -> ViewarrNormalize:
        """Get a ViewarrNormalize object matching the current viewer settings.

        Returns a normalization object that can be used with matplotlib to
        reproduce the same stretch settings you've dialed in interactively.

        Returns
        -------
        ViewarrNormalize
            Normalization object with current contrast, bias, and stretch settings.
        """
        return ViewarrNormalize(
            vmin=self.vmin,
            vmax=self.vmax,
            contrast=self.contrast,
            bias=self.bias,
            log=(self.stretch_mode == "log"),
            symmetric=(self.stretch_mode == "symmetric"),
        )

    def get_viewer_config(self) -> ViewerConfig:
        """Return the current widget state as a ViewerConfig."""
        return ViewerConfig(
            contrast=self.contrast,
            bias=self.bias,
            stretch=self.stretch_mode,
            zoom=self.zoom,
            cmap=self.colormap,
            colormap_reversed=self.colormap_reversed,
            vmin=self.vmin,
            vmax=self.vmax,
            xlim=self.xlim,
            ylim=self.ylim,
            rotation=self.rotation,
            pivot=self.pivot,
            show_pivot_marker=self.show_pivot_marker,
            markers=self.markers,
            on_shift_click=self._on_shift_click,
            overlay_message=self.overlay_message,
        )

    def plot_to_matplotlib(
        self, ax: "Axes", cmap: Optional[str] = None, **imshow_kwargs
    ) -> "Axes":
        """Plot the current view to a matplotlib axes.

        Renders the current 2D slice with the same normalization settings
        (contrast, bias, stretch) and rotation that are currently applied in
        the interactive viewer. Sets xlim/ylim to match the current viewport.

        Parameters
        ----------
        ax : matplotlib.axes.Axes
            The axes to plot on.
        cmap : str, optional
            Colormap name. If None, uses the viewer's current colormap.
        **imshow_kwargs
            Additional keyword arguments passed to ax.imshow().

        Returns
        -------
        matplotlib.axes.Axes
            The axes with the plotted image.

        Examples
        --------
        >>> import matplotlib.pyplot as plt
        >>> fig, ax = plt.subplots()
        >>> widget.plot_to_matplotlib(ax)
        >>> plt.show()
        """
        from matplotlib.transforms import Affine2D

        # Get the current slice data
        data = self.get_current_slice()

        # Create normalization matching viewer settings
        norm = self.get_normalization()

        # Determine colormap
        if cmap is None:
            viewer_cmap = self.colormap
            cmap = _COLORMAP_MAP.get(viewer_cmap, "gray")
            if self.colormap_reversed:
                # Append _r if not already reversed, or remove it if it is
                if cmap.endswith("_r"):
                    cmap = cmap[:-2]
                else:
                    cmap = cmap + "_r"

        # Set default imshow parameters
        imshow_defaults = {
            "origin": "lower",  # FITS convention: Y=0 at bottom
            "aspect": "equal",
        }
        imshow_defaults.update(imshow_kwargs)

        # Create rotation transform around pivot point
        # Note: Matplotlib rotates clockwise for positive angles, but viewarr
        # uses CCW (math convention), so we negate the angle
        rotation_deg = self.rotation
        pivot_x, pivot_y = self.pivot

        if abs(rotation_deg) > 0.01:
            # Build affine transform: rotate around pivot point
            # The transform is applied to data coordinates before rendering
            rotation_transform = (
                Affine2D().rotate_deg_around(pivot_x, pivot_y, -rotation_deg)
                + ax.transData
            )
            imshow_defaults["transform"] = rotation_transform

        # Plot the image
        ax.imshow(data, norm=norm, cmap=cmap, **imshow_defaults)

        # Set viewport limits
        xlim = self.xlim
        ylim = self.ylim

        if abs(rotation_deg) > 0.01:
            # Use viewport limits even when rotated
            if xlim[0] != xlim[1]:
                ax.set_xlim(xlim)
            if ylim[0] != ylim[1]:
                ax.set_ylim(ylim)

            # Hide tick labels when rotated since pixel coordinates become meaningless
            ax.set_xticklabels([])
            ax.set_yticklabels([])
        else:
            # No rotation - use viewport limits if set
            if xlim[0] != xlim[1]:
                ax.set_xlim(xlim)
            if ylim[0] != ylim[1]:
                ax.set_ylim(ylim)

        return ax


def create_viewer(
    width: Optional[int] = None,
    height: Optional[int] = None,
    viewer_config: Optional[ViewerConfig] = None,
    **kwargs: Any,
) -> ViewArrWidget:
    """Create the viewer widget with a given configuration.

    Args:
        width: Widget width in pixels (default: 800).
        height: Widget height in pixels (default: 600).
        viewer_config: Optional initial viewer state configuration.
        **kwargs: ViewerConfig fields to initialize state (see ViewerConfig).
            If provided alongside ``viewer_config``, these values override it.

    Returns:
        A Widget instance displaying the array.
    """
    if kwargs:
        if viewer_config is None:
            viewer_config = ViewerConfig(**kwargs)
        else:
            merged = asdict(viewer_config)
            merged.update(kwargs)
            viewer_config = ViewerConfig(**merged)

    widget = ViewArrWidget(viewer_config=viewer_config)
    if width is not None:
        widget.widget_width = width
    if height is not None:
        widget.widget_height = height

    return widget


def viewarr(
    arr: np.ndarray,
    width: Optional[int] = None,
    height: Optional[int] = None,
    viewer_config: Optional[ViewerConfig] = None,
    **kwargs: Any,
) -> ViewArrWidget:
    """Display a numpy array in an interactive viewer using
    `IPython.display.display` and `create_viewer` and return
    the resulting widget instance.

    Args:
        arr: A numpy array to display. Last two axes are treated as (y, x).
             Leading axes can be navigated with slice controls.
        width: Widget width in pixels (default: 800).
        height: Widget height in pixels (default: 600).
        viewer_config: Optional initial viewer state configuration.
        **kwargs: ViewerConfig fields to initialize state (see ViewerConfig).
            If provided alongside ``viewer_config``, these values override it.

    Returns:
        None
    """
    widget = create_viewer(
        width=width, height=height, viewer_config=viewer_config, **kwargs
    )
    widget.set_array(arr)
    return widget


def show(
    arr: np.ndarray,
    width: Optional[int] = None,
    height: Optional[int] = None,
    viewer_config: Optional[ViewerConfig] = None,
    **kwargs: Any,
):
    """Display a numpy array in an interactive viewer using
    `IPython.display.display` and `viewarr`.

    (For access to the widget instance, use `viewarr`.)

    Args:
        arr: A numpy array to display. Last two axes are treated as (y, x).
             Leading axes can be navigated with slice controls.
        width: Widget width in pixels (default: 800).
        height: Widget height in pixels (default: 600).
        viewer_config: Optional initial viewer state configuration.
        **kwargs: ViewerConfig fields to initialize state (see ViewerConfig).
            If provided alongside ``viewer_config``, these values override it.

    Returns:
        None
    """
    display(
        viewarr(
            arr,
            width=width,
            height=height,
            viewer_config=viewer_config,
            **kwargs,
        )
    )
