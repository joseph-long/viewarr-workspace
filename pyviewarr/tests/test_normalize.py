"""Tests for ViewarrNormalize and Widget functionality."""

import numpy as np
import pytest

from pyviewarr import ViewerConfig, ViewarrNormalize, ViewArrWidget


class TestViewarrNormalize:
    """Test suite for ViewarrNormalize class."""

    def test_identity_normalization(self):
        """Default settings (contrast=1, bias=0.5) should be identity for 0-1 input."""
        norm = ViewarrNormalize(vmin=0, vmax=1, contrast=1.0, bias=0.5)
        
        # Test scalar values
        assert abs(norm(0.0) - 0.0) < 1e-10
        assert abs(norm(0.5) - 0.5) < 1e-10
        assert abs(norm(1.0) - 1.0) < 1e-10

    def test_identity_array(self):
        """Test identity normalization with arrays."""
        norm = ViewarrNormalize(vmin=0, vmax=1, contrast=1.0, bias=0.5)
        data = np.array([0.0, 0.25, 0.5, 0.75, 1.0])
        result = norm(data)
        np.testing.assert_allclose(result, data, atol=1e-10)

    def test_scaling_to_range(self):
        """Test that data is scaled from vmin/vmax to 0-1."""
        norm = ViewarrNormalize(vmin=0, vmax=100, contrast=1.0, bias=0.5)
        
        assert abs(norm(0) - 0.0) < 1e-10
        assert abs(norm(50) - 0.5) < 1e-10
        assert abs(norm(100) - 1.0) < 1e-10

    def test_contrast_increase(self):
        """Higher contrast should spread values further from center."""
        norm = ViewarrNormalize(vmin=0, vmax=1, contrast=2.0, bias=0.5)
        
        # At contrast=2, the formula is: (x - 0.5) * 2 + 0.5 = 2x - 0.5
        # x=0   -> -0.5 clipped to 0
        # x=0.25 -> 0
        # x=0.5 -> 0.5
        # x=0.75 -> 1.0
        # x=1   -> 1.5 clipped to 1
        
        assert abs(norm(0.0) - 0.0) < 1e-10  # clipped
        assert abs(norm(0.25) - 0.0) < 1e-10
        assert abs(norm(0.5) - 0.5) < 1e-10
        assert abs(norm(0.75) - 1.0) < 1e-10
        assert abs(norm(1.0) - 1.0) < 1e-10  # clipped

    def test_contrast_decrease(self):
        """Lower contrast should compress values toward center."""
        norm = ViewarrNormalize(vmin=0, vmax=1, contrast=0.5, bias=0.5)
        
        # At contrast=0.5, the formula is: (x - 0.5) * 0.5 + 0.5 = 0.5x + 0.25
        # x=0 -> 0.25
        # x=0.5 -> 0.5
        # x=1 -> 0.75
        
        assert abs(norm(0.0) - 0.25) < 1e-10
        assert abs(norm(0.5) - 0.5) < 1e-10
        assert abs(norm(1.0) - 0.75) < 1e-10

    def test_bias_shift(self):
        """Changing bias should shift the midpoint."""
        # Bias=0.3: (x - 0.3) * 1 + 0.5 = x + 0.2
        norm = ViewarrNormalize(vmin=0, vmax=1, contrast=1.0, bias=0.3)
        
        assert abs(norm(0.0) - 0.2) < 1e-10
        assert abs(norm(0.3) - 0.5) < 1e-10
        assert abs(norm(0.8) - 1.0) < 1e-10  # clipped

    def test_log_stretch_endpoints(self):
        """Log stretch should map 0->0 and 1->1."""
        norm = ViewarrNormalize(vmin=0, vmax=1, contrast=1.0, bias=0.5, log=True)
        
        # At endpoints, log stretch with default contrast/bias should still give 0 and 1
        assert abs(norm(0.0) - 0.0) < 1e-10
        assert abs(norm(1.0) - 1.0) < 1e-10

    def test_log_stretch_compression(self):
        """Log stretch should expand low values and compress high values toward 1."""
        norm_linear = ViewarrNormalize(vmin=0, vmax=1, contrast=1.0, bias=0.5, log=False)
        norm_log = ViewarrNormalize(vmin=0, vmax=1, contrast=1.0, bias=0.5, log=True)

        # For low values, log stretch should give higher output than linear
        # (log expands the dynamic range for faint sources)
        low_val = 0.1
        assert norm_log(low_val) > norm_linear(low_val)

        # For the midpoint (0.5), log stretch should also give higher output
        # because log10(500+1)/3 ≈ 0.9 > 0.5
        mid_val = 0.5
        assert norm_log(mid_val) > norm_linear(mid_val)
    def test_log_stretch_formula(self):
        """Verify log stretch formula: log10(1000*x + 1) / log10(1000)."""
        norm = ViewarrNormalize(vmin=0, vmax=1, contrast=1.0, bias=0.5, log=True)
        
        x = 0.5
        # Expected: log10(1000*0.5 + 1) / log10(1000) = log10(501) / 3
        expected_stretched = np.log10(1000 * x + 1) / np.log10(1000)
        # Then apply contrast/bias: (stretched - 0.5) * 1 + 0.5 = stretched
        expected = expected_stretched
        
        result = norm(x)
        assert abs(result - expected) < 1e-10

    def test_symmetric_mode_centering(self):
        """Symmetric mode should center the scale on zero."""
        # Data ranges from -10 to 20, symmetric should use -20 to 20
        norm = ViewarrNormalize(vmin=-10, vmax=20, symmetric=True)
        
        # Zero should map to 0.5
        assert abs(norm(0.0) - 0.5) < 1e-10
        
        # -20 -> 0, 20 -> 1 (the symmetric range)
        assert abs(norm(-20) - 0.0) < 1e-10
        assert abs(norm(20) - 1.0) < 1e-10

    def test_symmetric_mode_bias_locked(self):
        """In symmetric mode, bias should be locked to 0.5."""
        # Even with bias=0.3, symmetric mode should use 0.5
        norm = ViewarrNormalize(vmin=-10, vmax=10, symmetric=True, bias=0.3)
        
        # Zero should still map to 0.5 (bias ignored)
        assert abs(norm(0.0) - 0.5) < 1e-10

    def test_symmetric_with_contrast(self):
        """Symmetric mode with contrast adjustment."""
        norm = ViewarrNormalize(vmin=-10, vmax=10, symmetric=True, contrast=2.0)
        
        # Zero should map to 0.5
        assert abs(norm(0.0) - 0.5) < 1e-10
        
        # With contrast=2: (x - 0.5) * 2 + 0.5
        # x=0.75 (7.5 in data) -> (0.75 - 0.5) * 2 + 0.5 = 1.0
        assert abs(norm(5.0) - 1.0) < 1e-10

    def test_clipping(self):
        """Values outside vmin/vmax should be clipped."""
        norm = ViewarrNormalize(vmin=0, vmax=100, clip=True)
        
        assert abs(norm(-50) - 0.0) < 1e-10
        assert abs(norm(150) - 1.0) < 1e-10

    def test_no_clipping(self):
        """With clip=False, values can exceed 0-1 range."""
        norm = ViewarrNormalize(vmin=0, vmax=100, clip=False)
        
        # -50 -> -0.5 normalized, then (-0.5 - 0.5) * 1 + 0.5 = -0.5
        result = norm(-50)
        assert result < 0

    def test_autoscale(self):
        """autoscale should set vmin/vmax from data."""
        norm = ViewarrNormalize()
        data = np.array([10, 20, 30, 40, 50])
        norm.autoscale(data)
        
        assert norm.vmin == 10
        assert norm.vmax == 50

    def test_autoscale_none(self):
        """autoscale_None should only set unset values."""
        norm = ViewarrNormalize(vmin=0)
        data = np.array([10, 20, 30])
        norm.autoscale_None(data)
        
        assert norm.vmin == 0  # Not changed
        assert norm.vmax == 30  # Set from data

    def test_scaled(self):
        """scaled() should return True when both vmin and vmax are set."""
        norm1 = ViewarrNormalize()
        assert not norm1.scaled()
        
        norm2 = ViewarrNormalize(vmin=0)
        assert not norm2.scaled()
        
        norm3 = ViewarrNormalize(vmin=0, vmax=1)
        assert norm3.scaled()

    def test_masked_array(self):
        """Masked values should remain masked."""
        norm = ViewarrNormalize(vmin=0, vmax=1)
        data = np.ma.array([0.0, 0.5, 1.0], mask=[False, True, False])
        result = norm(data)
        
        assert result.mask[1]  # Middle value should still be masked
        assert not result.mask[0]
        assert not result.mask[2]

    def test_nan_handling(self):
        """NaN values should be handled gracefully."""
        norm = ViewarrNormalize(vmin=0, vmax=1)
        data = np.array([0.0, np.nan, 1.0])
        result = norm(data)
        
        assert abs(result[0] - 0.0) < 1e-10
        assert np.isnan(result[1]) or np.ma.is_masked(result[1])
        assert abs(result[2] - 1.0) < 1e-10

    def test_2d_array(self):
        """Test with 2D array input."""
        norm = ViewarrNormalize(vmin=0, vmax=100, contrast=1.0, bias=0.5)
        data = np.array([[0, 25], [50, 100]])
        expected = np.array([[0.0, 0.25], [0.5, 1.0]])
        result = norm(data)
        np.testing.assert_allclose(result, expected, atol=1e-10)


class TestWidget:
    """Test suite for Widget class (Python-only tests, no frontend)."""

    def test_widget_creation(self):
        """Widget should be creatable with default values."""
        widget = ViewArrWidget()
        assert widget.contrast == 1.0
        assert widget.bias == 0.5
        assert widget.stretch_mode == "linear"
        assert widget.xlim == (0.0, 0.0)
        assert widget.ylim == (0.0, 0.0)
        assert widget.overlay_message == ""
        assert widget.markers == []

    def test_set_array_2d(self):
        """set_array should work with 2D arrays."""
        widget = ViewArrWidget()
        data = np.random.rand(100, 200).astype(np.float64)
        widget.set_array(data)
        
        assert widget.shape == [100, 200]
        assert widget.image_height == 100
        assert widget.image_width == 200
        assert widget.current_slice_indices == []

    def test_set_array_3d(self):
        """set_array should work with 3D arrays."""
        widget = ViewArrWidget()
        data = np.random.rand(10, 100, 200).astype(np.float64)
        widget.set_array(data)
        
        assert widget.shape == [10, 100, 200]
        assert widget.image_height == 100
        assert widget.image_width == 200
        assert widget.current_slice_indices == [0]

    def test_set_array_4d(self):
        """set_array should work with 4D arrays."""
        widget = ViewArrWidget()
        data = np.random.rand(5, 10, 100, 200).astype(np.float64)
        widget.set_array(data)
        
        assert widget.shape == [5, 10, 100, 200]
        assert widget.image_height == 100
        assert widget.image_width == 200
        assert widget.current_slice_indices == [0, 0]

    def test_set_array_1d_raises(self):
        """set_array should raise for 1D arrays."""
        widget = ViewArrWidget()
        data = np.random.rand(100)
        
        with pytest.raises(ValueError, match="at least 2 dimensions"):
            widget.set_array(data)

    def test_get_current_slice_2d(self):
        """get_current_slice should return the array for 2D input."""
        widget = ViewArrWidget()
        data = np.random.rand(100, 200).astype(np.float64)
        widget.set_array(data)
        
        slice_data = widget.get_current_slice()
        np.testing.assert_array_equal(slice_data, data)

    def test_get_current_slice_3d(self):
        """get_current_slice should return the correct slice for 3D input."""
        widget = ViewArrWidget()
        data = np.arange(5 * 10 * 20).reshape(5, 10, 20).astype(np.float64)
        widget.set_array(data)
        
        # Default slice is [0]
        slice_data = widget.get_current_slice()
        np.testing.assert_array_equal(slice_data, data[0])
        
        # Change slice
        widget.current_slice_indices = [2]
        slice_data = widget.get_current_slice()
        np.testing.assert_array_equal(slice_data, data[2])

    def test_get_normalization(self):
        """get_normalization should return ViewarrNormalize with current settings."""
        widget = ViewArrWidget()
        widget.contrast = 2.0
        widget.bias = 0.3
        widget.stretch_mode = "log"
        widget.vmin = 10.0
        widget.vmax = 1000.0
        
        norm = widget.get_normalization()
        
        assert isinstance(norm, ViewarrNormalize)
        assert norm.contrast == 2.0
        assert norm.bias == 0.3
        assert norm.log is True
        assert norm.symmetric is False
        assert norm.vmin == 10.0
        assert norm.vmax == 1000.0

    def test_get_normalization_symmetric(self):
        """get_normalization should handle symmetric mode."""
        widget = ViewArrWidget()
        widget.stretch_mode = "symmetric"
        
        norm = widget.get_normalization()
        
        assert norm.log is False
        assert norm.symmetric is True

    def test_dtype_conversion(self):
        """Widget should handle different numpy dtypes."""
        widget = ViewArrWidget()
        
        # Test various dtypes
        for dtype in [np.int8, np.uint8, np.int16, np.uint16, 
                      np.int32, np.uint32, np.float32, np.float64]:
            data = np.array([[1, 2], [3, 4]], dtype=dtype)
            widget.set_array(data)
            assert widget.image_height == 2
            assert widget.image_width == 2

    def test_viewer_config_shift_click_callback_and_overlay(self):
        """ViewerConfig callback and overlay message should be applied to widget."""
        clicks = []

        def on_shift_click(x, y):
            clicks.append((x, y))

        config = ViewerConfig(
            on_shift_click=on_shift_click,
            overlay_message="Shift-click stores points",
        )
        widget = ViewArrWidget(viewer_config=config)

        assert widget.overlay_message == "Shift-click stores points"
        widget._shift_click_event = {"x": 12.25, "y": 4.75, "token": 1}
        assert clicks == [(12.25, 4.75)]

    def test_viewer_config_markers_apply_to_widget(self):
        """ViewerConfig marker list should initialize the widget marker trait."""
        config = ViewerConfig(markers=[(1.25, 2.5), (10.0, 12.0)])
        widget = ViewArrWidget(viewer_config=config)
        assert widget.markers == [(1.25, 2.5), (10.0, 12.0)]

    def test_viewer_config_to_js_dict_excludes_python_only_fields(self):
        """Python-only config fields should not be sent to JS viewer state."""
        config = ViewerConfig(
            zoom=2.0,
            markers=[(1.5, 2.5)],
            on_shift_click=lambda x, y: None,
            overlay_message="Shift-click callback active",
        )
        js_state = config.to_js_dict()

        assert js_state["zoom"] == 2.0
        assert js_state["markers"] == [(1.5, 2.5)]
        assert "on_shift_click" not in js_state
        assert "overlay_message" not in js_state


class TestWidgetMatplotlib:
    """Test matplotlib integration (requires matplotlib)."""

    @pytest.fixture
    def mock_axes(self):
        """Create a mock axes object for testing without display."""
        try:
            import matplotlib.pyplot as plt
            fig, ax = plt.subplots()
            yield ax
            plt.close(fig)
        except ImportError:
            pytest.skip("matplotlib not installed")

    def test_plot_to_matplotlib(self, mock_axes):
        """plot_to_matplotlib should work with basic data."""
        widget = ViewArrWidget()
        data = np.random.rand(100, 200).astype(np.float64)
        widget.set_array(data)
        widget.vmin = 0.0
        widget.vmax = 1.0
        
        result = widget.plot_to_matplotlib(mock_axes)
        
        # Should return the same axes
        assert result is mock_axes
        # Should have created an image
        assert len(mock_axes.images) == 1

    def test_plot_to_matplotlib_with_cmap(self, mock_axes):
        """plot_to_matplotlib should accept custom colormap."""
        widget = ViewArrWidget()
        data = np.random.rand(50, 50).astype(np.float64)
        widget.set_array(data)
        widget.vmin = 0.0
        widget.vmax = 1.0
        
        widget.plot_to_matplotlib(mock_axes, cmap='viridis')
        
        assert len(mock_axes.images) == 1

    def test_plot_to_matplotlib_sets_limits(self, mock_axes):
        """plot_to_matplotlib should set xlim/ylim when specified."""
        widget = ViewArrWidget()
        data = np.random.rand(100, 200).astype(np.float64)
        widget.set_array(data)
        widget.vmin = 0.0
        widget.vmax = 1.0
        widget.xlim = (50.0, 150.0)
        widget.ylim = (25.0, 75.0)
        
        widget.plot_to_matplotlib(mock_axes)
        
        xlim = mock_axes.get_xlim()
        ylim = mock_axes.get_ylim()
        
        assert abs(xlim[0] - 50.0) < 1e-10
        assert abs(xlim[1] - 150.0) < 1e-10
        assert abs(ylim[0] - 25.0) < 1e-10
        assert abs(ylim[1] - 75.0) < 1e-10
