import { test, expect } from "@jupyterlab/galata";

// Load a small 3D cube so the in-canvas slice slider + play/speed controls appear.
const CUBE_CELL = `import numpy as np
import pyviewarr

arr = np.arange(2 * 4 * 4, dtype=np.float32).reshape(2, 4, 4)
w = pyviewarr.show(arr, width=360, height=260)
w`;

test("pyviewarr cube widget renders the in-canvas slice controls", async ({
	page
}) => {
	// Create the notebook in-process (more robust than opening a file by path).
	await page.notebook.createNew();
	await page.notebook.setCell(0, "code", CUBE_CELL);
	await page.notebook.run();

	const widget = page.locator(".jp-OutputArea-output .pyviewarr").first();
	await expect(widget).toBeVisible();

	// The cube slice + play controls now live inside the egui canvas (they are
	// no longer DOM elements), so we assert the WASM viewer canvas renders.
	const canvas = widget.locator(".pyviewarr-container canvas").first();
	await expect(canvas).toBeVisible();

	// The slice/play bar is drawn inside the canvas and cannot be asserted on via
	// the DOM, so save a screenshot for human verification: the top-center slice
	// slider + play/speed controls should be visible over the image.
	await page.waitForTimeout(2000); // let egui paint the controls + first slice
	await widget.screenshot({ path: "tests/e2e/screenshots/pyviewarr-cube.png" });
});
