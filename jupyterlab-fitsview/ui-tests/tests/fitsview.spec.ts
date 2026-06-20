import { expect, test } from '@jupyterlab/galata';

/**
 * Note: These tests require a test.fits file to be present.
 * The test file should be created before running tests.
 */

test.describe('FITS Viewer Extension', () => {
  test.beforeEach(async ({ page }) => {
    // Create a test FITS file using Python
    await page.menu.clickMenuItem('File>New>Notebook');
    await page.getByRole('button', { name: 'Select' }).click();

    await page.notebook.setCell(
      0,
      'code',
      `from astropy.io import fits
import numpy as np

# Create a simple test FITS file
data = np.random.random((2, 50, 50)).astype(np.float32)
hdu = fits.PrimaryHDU(data)
hdu.header['OBJECT'] = 'Test Object'
hdul = fits.HDUList([hdu])
hdul.writeto('test.fits', overwrite=True)

# A many-plane cube for the scrub/latency test
fits.PrimaryHDU(
    np.random.random((100, 64, 64)).astype(np.float32)
).writeto('cube.fits', overwrite=True)
print('Created test.fits')`
    );

    await page.notebook.run();
    await page.waitForTimeout(500);

    // Close the notebook
    await page.menu.clickMenuItem('File>Close Tab');
    await page.getByRole('button', { name: 'Discard' }).click();
  });

  test('should open FITS file in viewer', async ({ page }) => {
    // Open the FITS file from file browser
    await page.filebrowser.open('test.fits');

    // Wait for the FITS viewer to load
    const viewer = page.getByRole('main').locator('.jp-FITSViewer');
    await expect(viewer).toBeVisible();

    // Check that metadata is displayed
    await expect(viewer.locator('.jp-FITSViewer-hduBar')).toContainText(
      'PRIMARY'
    );
  });

  test('should display HDU information', async ({ page }) => {
    await page.filebrowser.open('test.fits');

    const viewer = page.getByRole('main').locator('.jp-FITSViewer');
    await expect(viewer).toBeVisible();
    const hduBar = viewer.locator('.jp-FITSViewer-hduBar');

    // Check HDU info is shown
    await expect(hduBar.locator('text=f32')).toBeVisible();
  });

  test('should render the viewer canvas for a cube', async ({ page }) => {
    await page.filebrowser.open('test.fits');

    const viewer = page.getByRole('main').locator('.jp-FITSViewer');
    await expect(viewer).toBeVisible();

    // Check that metadata is displayed
    await expect(viewer.locator('.jp-FITSViewer-hduBar')).toContainText(
      'PRIMARY'
    );

    // The slice + play controls for the cube's leading axis now live inside the
    // egui canvas (no longer DOM elements), so we assert the viewer canvas for
    // the first HDU renders.
    const canvas = viewer.locator('.jp-FITSViewer-viewerContainer canvas').first();
    await expect(canvas).toBeVisible();

    // The in-canvas slice/play bar can't be asserted on via the DOM, so save a
    // screenshot for human verification. The test cube is 3D (2, 50, 50), so the
    // top-center slice slider + play/speed controls should be visible.
    await page.waitForTimeout(2000); // let egui paint the controls + first slice
    await viewer.screenshot({
      path: 'screenshots/fitsview-cube.png'
    });
  });

  test('coalesces slice requests under latency and shows the index overlay', async ({
    page
  }) => {
    // cube.fits (100 planes) is created in beforeEach.
    // Add 300ms latency to every slice fetch and count them. (Slices are plain
    // HTTP GETs, so page.route can intercept them.)
    let sliceRequests = 0;
    await page.route('**/fitsview/slice*', async route => {
      sliceRequests += 1;
      await new Promise(r => setTimeout(r, 300));
      await route.continue();
    });

    await page.filebrowser.open('cube.fits');
    const viewer = page.getByRole('main').locator('.jp-FITSViewer');
    await expect(viewer).toBeVisible();
    const canvas = viewer
      .locator('.jp-FITSViewer-viewerContainer canvas')
      .first();
    await expect(canvas).toBeVisible();
    await page.waitForTimeout(1500); // initial slice loads
    const baseline = sliceRequests;

    // Fast-drag the scrubber across the whole axis. The scrubber track is in the
    // top bar (~13px down); start past the play+speed controls on the left and
    // sweep to just before the index readout on the right.
    const box = (await canvas.boundingBox())!;
    const trackY = box.y + 13;
    const startX = box.x + 100;
    const endX = box.x + box.width - 90;
    await page.mouse.move(startX, trackY);
    await page.mouse.down();
    const steps = 16;
    for (let i = 1; i <= steps; i++) {
      await page.mouse.move(startX + ((endX - startX) * i) / steps, trackY);
    }
    await page.mouse.up();

    // The displayed frame now lags the handle -> the big centered index overlay
    // is showing. Capture it before the in-flight slice lands.
    await viewer.screenshot({
      path: 'screenshots/fitsview-scrub-overlay.png'
    });

    // Let the coalesced fetch settle.
    await page.waitForTimeout(1500);

    // The drag swept ~100 frames, but coalescing keeps at most one request in
    // flight, so only a few slices were actually fetched (a request per latency
    // window, not one per frame).
    const dragRequests = sliceRequests - baseline;
    expect(dragRequests).toBeGreaterThan(0);
    expect(dragRequests).toBeLessThan(10);
  });
});
