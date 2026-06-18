from astropy.io import fits
import math
import numpy as np
import pathlib
import sys


if __name__ == "__main__":
    dest = pathlib.Path(sys.argv[1])
    if not dest.is_dir():
        raise RuntimeError(f"Make sure the directory {sys.argv[1]} exists for the destination")
    rng = np.random.default_rng(seed=0)
    # cases to cover:
    # - simple 2D image
    simple_2d = rng.random((16, 16))
    fits.PrimaryHDU(simple_2d).writeto(dest / 'simple_2d.fits', overwrite=True)
    # - simple 2D image (integers)
    simple_2d_uints = (1000 * rng.random((16, 16))).astype(np.uint16)
    fits.PrimaryHDU(simple_2d_uints).writeto(dest / 'simple_2d_uints.fits', overwrite=True)
    # - simple 2D image > 5MB
    min_bytes = 5.5e6
    bytes_per_double = 4
    npix_per_side = math.ceil(math.sqrt(min_bytes / bytes_per_double))
    big_2d = rng.random((npix_per_side, npix_per_side), dtype=np.float64)
    assert big_2d.nbytes >= min_bytes
    fits.PrimaryHDU(big_2d).writeto(dest / 'big_2d.fits', overwrite=True)
    # - 3D data cube
    simple_3d = rng.random((5, 16, 16))
    fits.PrimaryHDU(simple_3d).writeto(dest / 'simple_3d.fits', overwrite=True)
    # - 3D data cube where 1 plane > 5 MB
    big_3d = rng.random((5, npix_per_side, npix_per_side), dtype=np.float64)
    assert big_3d[0].nbytes >= min_bytes
    fits.PrimaryHDU(big_3d).writeto(dest / 'big_3d.fits', overwrite=True)
    # - 4D hypercube
    simple_4d = rng.random((4, 5, 16, 16))
    fits.PrimaryHDU(simple_4d).writeto(dest / 'simple_4d.fits', overwrite=True)
    # - file with no image extensions
    table_hdu = fits.TableHDU(np.ones((10,), dtype=[('x', float), ('y', float)]), name='TABLE')
    table_hdu.writeto(dest / 'no_image.fits', overwrite=True)
    # - file with multiple extensions
    fits.HDUList([
        fits.PrimaryHDU(),
        fits.ImageHDU(simple_4d, name='FOURDEE'),
        table_hdu,
    ]).writeto(dest / 'multi_ext.fits', overwrite=True)
    # - file with multiple *image* extensions
    fits.HDUList([
        fits.PrimaryHDU(),
        fits.ImageHDU(simple_4d, name='FOURDEE'),
        fits.ImageHDU(simple_3d, name='THREEDEE')
    ]).writeto(dest / 'multi_image_ext.fits', overwrite=True)
