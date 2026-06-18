"""
API handlers for FITS file operations.
"""

import json
import tornado
from enum import Enum
from jupyter_server.base.handlers import APIHandler, JupyterHandler
from jupyter_server.utils import url_path_join
from astropy.io import fits
import numpy as np


class ArrayType(str, Enum):
    """
    Enum mapping numpy dtypes to Rust-style type specifiers.

    Uses dtype.kind and dtype.itemsize to unambiguously determine the type,
    avoiding string comparison issues with numpy's various dtype representations.

    Values use Rust-style type names for consistency across the wire protocol.
    """
    INT8 = "i8"
    UINT8 = "u8"
    INT16 = "i16"
    UINT16 = "u16"
    INT32 = "i32"
    UINT32 = "u32"
    INT64 = "i64"
    UINT64 = "u64"
    FLOAT32 = "f32"
    FLOAT64 = "f64"


def numpy_dtype_to_array_type(dtype: np.dtype) -> ArrayType:
    """
    Map a numpy dtype to an ArrayType based on kind and itemsize.

    dtype.kind values:
      'b' - boolean
      'i' - signed integer
      'u' - unsigned integer
      'f' - floating-point
      'c' - complex floating-point
      'S', 'a' - byte string
      'U' - unicode string
      'V' - void (raw data)
    """
    kind = dtype.kind
    size = dtype.itemsize

    if kind == 'i':  # Signed integer
        if size == 1:
            return ArrayType.INT8
        elif size == 2:
            return ArrayType.INT16
        elif size == 4:
            return ArrayType.INT32
        elif size == 8:
            return ArrayType.INT64
    elif kind == 'u':  # Unsigned integer
        if size == 1:
            return ArrayType.UINT8
        elif size == 2:
            return ArrayType.UINT16
        elif size == 4:
            return ArrayType.UINT32
        elif size == 8:
            return ArrayType.UINT64
    elif kind == 'f':  # Floating-point
        if size == 4:
            return ArrayType.FLOAT32
        elif size == 8:
            return ArrayType.FLOAT64
    elif kind == 'b':  # Boolean - treat as uint8
        return ArrayType.UINT8

    # Default to float64 for unsupported types
    return ArrayType.FLOAT64


class FITSMetadataHandler(APIHandler):
    """Handler for retrieving FITS file metadata (headers, dimensions)."""

    @tornado.web.authenticated
    async def get(self):
        path = self.get_argument('path')

        # Validate path exists via Contents API
        cm = self.contents_manager
        try:
            await cm.get(path, content=False)
        except Exception as e:
            self.set_status(404)
            self.finish(json.dumps({'error': f'File not found: {path}'}))
            return

        # Get filesystem path for astropy
        os_path = cm._get_os_path(path)

        try:
            with fits.open(os_path) as hdul:
                hdus = []
                for i, hdu in enumerate(hdul):
                    # Use repr() to get the raw 80-column card format with newlines
                    # str() returns an object description, repr() returns the actual content
                    header_str = repr(hdu.header)

                    hdu_info = {
                        'index': i,
                        'name': hdu.name,
                        'type': hdu.__class__.__name__,
                        'header': header_str,  # Raw 80-column format string
                    }
                    if hdu.data is not None:
                        hdu_info['shape'] = list(hdu.data.shape)
                        # Use ArrayType enum for consistent type representation
                        hdu_info['arrayType'] = numpy_dtype_to_array_type(hdu.data.dtype).value
                    else:
                        hdu_info['shape'] = None
                        hdu_info['arrayType'] = None
                    hdus.append(hdu_info)

                result = {
                    'path': path,
                    'hdus': hdus
                }
                self.finish(json.dumps(result))
        except Exception as e:
            self.set_status(500)
            self.finish(json.dumps({'error': f'Error reading FITS file: {str(e)}'}))


class FITSSliceHandler(JupyterHandler):
    """Handler for retrieving data slices from FITS files.

    Uses JupyterHandler instead of APIHandler to support binary responses.

    The slices parameter uses NumPy/Python conventions:
    - Zero-indexed
    - Axis order matches NumPy (e.g., for 3D data: z,y,x or depth,row,col)
    - Half-open intervals [start, stop) with exclusive upper bound
    - Format: "start:stop,start:stop,..." for each axis
    """

    @tornado.web.authenticated
    async def get(self):
        path = self.get_argument('path')
        hdu = int(self.get_argument('hdu', 0))
        slices_str = self.get_argument('slices')  # e.g., "0:10,5:15" for 2D

        # Parse slices parameter
        try:
            slice_tuples = []
            for s in slices_str.split(','):
                parts = s.strip().split(':')
                if len(parts) != 2:
                    raise ValueError(f"Invalid slice format: '{s}'. Expected 'start:stop'.")
                start, stop = int(parts[0]), int(parts[1])
                if start < 0 or stop < 0:
                    raise ValueError(f"Negative indices not supported: '{s}'")
                if start >= stop:
                    raise ValueError(f"Start must be less than stop: '{s}'")
                slice_tuples.append((start, stop))
        except ValueError as e:
            self.set_status(400)
            self.finish(json.dumps({'error': str(e)}))
            return

        # Validate path exists via Contents API
        cm = self.contents_manager
        try:
            await cm.get(path, content=False)
        except Exception as e:
            self.set_status(404)
            self.finish(json.dumps({'error': f'File not found: {path}'}))
            return

        # Get filesystem path for astropy
        os_path = cm._get_os_path(path)

        try:
            with fits.open(os_path) as hdul:
                if hdu >= len(hdul):
                    self.set_status(400)
                    self.finish(json.dumps({
                        'error': f'HDU index {hdu} out of range (file has {len(hdul)} HDUs)'
                    }))
                    return

                data = hdul[hdu].data
                if data is None:
                    self.set_status(400)
                    self.finish(json.dumps({
                        'error': f'HDU {hdu} has no data'
                    }))
                    return

                # Validate number of slice dimensions matches data dimensions
                if len(slice_tuples) != len(data.shape):
                    self.set_status(400)
                    self.finish(json.dumps({
                        'error': f'Number of slice dimensions ({len(slice_tuples)}) does not match '
                                 f'data dimensions ({len(data.shape)}). Data shape: {list(data.shape)}'
                    }))
                    return

                # Validate slice bounds for each axis
                for axis, ((start, stop), size) in enumerate(zip(slice_tuples, data.shape)):
                    if stop > size:
                        self.set_status(400)
                        self.finish(json.dumps({
                            'error': f'Slice [{start}:{stop}] on axis {axis} out of bounds '
                                     f'for dimension size {size}. Data shape: {list(data.shape)}'
                        }))
                        return

                # Build the slice tuple and extract data
                numpy_slices = tuple(slice(start, stop) for start, stop in slice_tuples)
                slice_data = data[numpy_slices]

                # Convert to little-endian for JavaScript TypedArray compatibility
                le_dtype = slice_data.dtype.newbyteorder('<')
                slice_bytes = slice_data.astype(le_dtype).tobytes()

                # Use ArrayType enum for consistent type representation
                array_type = numpy_dtype_to_array_type(slice_data.dtype)

                self.set_header('Content-Type', 'application/octet-stream')
                self.set_header('X-FITS-Shape', json.dumps(list(slice_data.shape)))
                self.set_header('X-FITS-Type', array_type.value)
                self.finish(slice_bytes)

        except Exception as e:
            self.set_status(500)
            self.finish(json.dumps({'error': f'Error reading FITS data: {str(e)}'}))


def setup_handlers(web_app):
    """Register FITS API handlers."""
    host_pattern = '.*$'
    base_url = web_app.settings['base_url']

    handlers = [
        (url_path_join(base_url, 'fitsview', 'metadata'), FITSMetadataHandler),
        (url_path_join(base_url, 'fitsview', 'slice'), FITSSliceHandler),
    ]
    web_app.add_handlers(host_pattern, handlers)
