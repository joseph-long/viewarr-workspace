"""Tests for FITS API handlers."""

import json

import numpy as np
import pytest
from astropy.io import fits


@pytest.fixture
def fits_file(jp_root_dir):
    """Create a test FITS file with multiple HDUs."""
    # Primary HDU with float32 data
    primary_data = np.arange(100, dtype=np.float32).reshape(10, 10)
    primary_hdu = fits.PrimaryHDU(primary_data)
    primary_hdu.header["OBJECT"] = "Test Object"
    primary_hdu.header["TELESCOP"] = "Test Telescope"

    # Image extension with int16 data
    int16_data = np.arange(50, dtype=np.int16).reshape(5, 10)
    image_hdu = fits.ImageHDU(int16_data, name="SCI")
    image_hdu.header["EXTNAME"] = "SCI"

    # Table extension
    col1 = fits.Column(name="ID", format="J", array=np.array([1, 2, 3]))
    col2 = fits.Column(name="NAME", format="10A", array=np.array(["a", "b", "c"]))
    table_hdu = fits.BinTableHDU.from_columns([col1, col2], name="TABLE")

    hdul = fits.HDUList([primary_hdu, image_hdu, table_hdu])

    fits_path = jp_root_dir / "test_data.fits"
    hdul.writeto(fits_path, overwrite=True)
    hdul.close()

    return "test_data.fits"


@pytest.fixture
def float64_fits_file(jp_root_dir):
    """Create a test FITS file with float64 data."""
    data = np.array([[1.5, 2.5], [3.5, 4.5]], dtype=np.float64)
    hdu = fits.PrimaryHDU(data)
    hdul = fits.HDUList([hdu])

    fits_path = jp_root_dir / "float64_test.fits"
    hdul.writeto(fits_path, overwrite=True)
    hdul.close()

    return "float64_test.fits"


@pytest.fixture
def cube_fits_file(jp_root_dir):
    """Create a test FITS file with 3D data cube."""
    # 3D cube: 4 planes of 5x6 data
    data = np.arange(120, dtype=np.float32).reshape(4, 5, 6)
    hdu = fits.PrimaryHDU(data)
    hdul = fits.HDUList([hdu])

    fits_path = jp_root_dir / "cube_test.fits"
    hdul.writeto(fits_path, overwrite=True)
    hdul.close()

    return "cube_test.fits"


class TestMetadataHandler:
    """Tests for the FITSMetadataHandler."""

    async def test_get_metadata(self, jp_fetch, fits_file):
        """Test retrieving metadata from a FITS file."""
        response = await jp_fetch("fitsview", "metadata", params={"path": fits_file})

        assert response.code == 200
        data = json.loads(response.body)

        assert data["path"] == fits_file

        # Check primary HDU
        hdus = data["hdus"]
        assert len(hdus) == 3

        primary = hdus[0]
        assert primary["index"] == 0
        assert primary["name"] == "PRIMARY"
        assert primary["type"] == "PrimaryHDU"
        assert primary["shape"] == [10, 10]
        # arrayType is now a Rust-style type specifier
        assert primary["arrayType"] == "f32"
        # header is returned as raw 80-column FITS format string
        assert "OBJECT" in primary["header"]
        assert "Test Object" in primary["header"]
        assert "TELESCOP" in primary["header"]
        assert "Test Telescope" in primary["header"]

        # Check image extension
        sci = hdus[1]
        assert sci["index"] == 1
        assert sci["name"] == "SCI"
        assert sci["type"] == "ImageHDU"
        assert sci["shape"] == [5, 10]
        assert sci["arrayType"] == "i16"

        # Check table extension
        table = hdus[2]
        assert table["index"] == 2
        assert table["name"] == "TABLE"
        assert table["type"] == "BinTableHDU"

    async def test_metadata_file_not_found(self, jp_fetch):
        """Test that 404 is returned for non-existent files."""
        response = await jp_fetch(
            "fitsview",
            "metadata",
            params={"path": "nonexistent.fits"},
            raise_error=False,
        )

        assert response.code == 404
        data = json.loads(response.body)
        assert "error" in data
        assert "not found" in data["error"].lower()


class TestSliceHandler:
    """Tests for the FITSSliceHandler."""

    async def test_get_slice_float32(self, jp_fetch, fits_file):
        """Test retrieving a slice of float32 data."""
        # 2D data with shape [10, 10], slice rows 0:2, cols 0:3
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": fits_file, "hdu": "0", "slices": "0:2,0:3"},
        )

        assert response.code == 200
        assert response.headers["Content-Type"] == "application/octet-stream"

        # Check shape header
        shape = json.loads(response.headers["X-FITS-Shape"])
        assert shape == [2, 3]

        # Check type header - should be f32 (Rust-style type specifier)
        array_type = response.headers["X-FITS-Type"]
        assert array_type == "f32"

        # Verify data content (data is sent as little-endian)
        data = np.frombuffer(response.body, dtype="<f4").reshape(2, 3)
        expected = np.arange(100, dtype=np.float32).reshape(10, 10)[0:2, 0:3]
        np.testing.assert_array_almost_equal(data, expected)

    async def test_get_slice_int16(self, jp_fetch, fits_file):
        """Test retrieving a slice of int16 data."""
        # HDU 1 has shape [5, 10], slice rows 1:3, cols 2:6
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": fits_file, "hdu": "1", "slices": "1:3,2:6"},
        )

        assert response.code == 200

        # Check type header - should be i16 (Rust-style type specifier)
        array_type = response.headers["X-FITS-Type"]
        assert array_type == "i16"

        # Verify data content (data is sent as little-endian)
        shape = json.loads(response.headers["X-FITS-Shape"])
        assert shape == [2, 4]

        data = np.frombuffer(response.body, dtype="<i2").reshape(2, 4)
        expected = np.arange(50, dtype=np.int16).reshape(5, 10)[1:3, 2:6]
        np.testing.assert_array_equal(data, expected)

    async def test_get_slice_float64(self, jp_fetch, float64_fits_file):
        """Test retrieving float64 data preserves dtype."""
        # 2D data with shape [2, 2], slice entire array
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": float64_fits_file, "hdu": "0", "slices": "0:2,0:2"},
        )

        assert response.code == 200
        
        # Check type header - should be f64 (Rust-style type specifier)
        array_type = response.headers["X-FITS-Type"]
        assert array_type == "f64"

        data = np.frombuffer(response.body, dtype="<f8").reshape(2, 2)
        expected = np.array([[1.5, 2.5], [3.5, 4.5]], dtype=np.float64)
        np.testing.assert_array_almost_equal(data, expected)

    async def test_get_slice_3d_cube(self, jp_fetch, cube_fits_file):
        """Test retrieving a slice from a 3D data cube."""
        # 3D data with shape [4, 5, 6], slice planes 1:3, rows 0:2, cols 2:5
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": cube_fits_file, "hdu": "0", "slices": "1:3,0:2,2:5"},
        )

        assert response.code == 200

        # Check shape header
        shape = json.loads(response.headers["X-FITS-Shape"])
        assert shape == [2, 2, 3]

        # Verify data content (using type header for reference, but hardcode dtype for parsing)
        array_type = response.headers["X-FITS-Type"]
        assert array_type == "f32"
        data = np.frombuffer(response.body, dtype="<f4").reshape(2, 2, 3)
        expected = np.arange(120, dtype=np.float32).reshape(4, 5, 6)[1:3, 0:2, 2:5]
        np.testing.assert_array_almost_equal(data, expected)

    async def test_slice_out_of_bounds(self, jp_fetch, fits_file):
        """Test that out-of-bounds slices return 400."""
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": fits_file, "hdu": "0", "slices": "8:15,8:15"},
            raise_error=False,
        )

        assert response.code == 400
        data = json.loads(response.body)
        assert "error" in data
        assert "out of bounds" in data["error"].lower()

    async def test_slice_dimension_mismatch(self, jp_fetch, fits_file):
        """Test that wrong number of slice dimensions returns 400."""
        # HDU 0 is 2D but we provide 3 slice dimensions
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": fits_file, "hdu": "0", "slices": "0:2,0:2,0:2"},
            raise_error=False,
        )

        assert response.code == 400
        data = json.loads(response.body)
        assert "error" in data
        assert "dimensions" in data["error"].lower()

    async def test_slice_invalid_format(self, jp_fetch, fits_file):
        """Test that invalid slice format returns 400."""
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": fits_file, "hdu": "0", "slices": "0:2:1,0:3"},  # step not supported
            raise_error=False,
        )

        assert response.code == 400
        data = json.loads(response.body)
        assert "error" in data

    async def test_slice_invalid_hdu(self, jp_fetch, fits_file):
        """Test that invalid HDU index returns 400."""
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": fits_file, "hdu": "99", "slices": "0:1,0:1"},
            raise_error=False,
        )

        assert response.code == 400
        data = json.loads(response.body)
        assert "error" in data
        assert "out of range" in data["error"].lower()

    async def test_slice_table_hdu_error(self, jp_fetch, fits_file):
        """Test that requesting a slice from a table HDU returns an error."""
        # HDU 2 is the table extension - tables have no standard shape
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": fits_file, "hdu": "2", "slices": "0:1,0:1"},
            raise_error=False,
        )

        assert response.code == 400
        data = json.loads(response.body)
        assert "error" in data

    async def test_slice_file_not_found(self, jp_fetch):
        """Test that 404 is returned for non-existent files."""
        response = await jp_fetch(
            "fitsview",
            "slice",
            params={"path": "nonexistent.fits", "hdu": "0", "slices": "0:1,0:1"},
            raise_error=False,
        )

        assert response.code == 404
        data = json.loads(response.body)
        assert "error" in data
