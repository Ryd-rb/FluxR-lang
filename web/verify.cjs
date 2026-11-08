const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const http = require('node:http');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE ?? 'playwright-core');

async function main() {
    const [native, browserPath] = process.argv.slice(2);
    if (!native || !browserPath) throw new Error('Usage: node web/verify.cjs NATIVE_PROGRAM BROWSER_EXE');
    const [bytes, replay] = await Promise.all([fs.readFile(native), fs.readFile(path.join(__dirname, 'well.replay.json'), 'utf8')]);
    const server = http.createServer(async (req, res) => {
        const name = req.url === '/' ? 'index.html' : req.url.slice(1);
        if (!/^(index\.html|editor\.js|well\.fluxr|pkg\/fluxr(_bg)?\.(js|wasm))$/.test(name)) {
            res.writeHead(404).end();
            return;
        }
        try {
            const mime = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };
            res.writeHead(200, { 'Content-Type': mime[path.extname(name)] ?? 'text/plain' });
            res.end(await fs.readFile(path.join(__dirname, name)));
        } catch {
            res.end();
        }
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    let browser;
    try {
        browser = await chromium.launch({ executablePath: browserPath, headless: true });
        const page = await browser.newPage();
        await page.goto(`http://127.0.0.1:${server.address().port}`);
        await page.waitForFunction(() => window.sceneEditor?.inspect().presented === 1);
        const initial = await page.evaluate(replay => ({
            bytes: window.sceneEditor.encode(), assertions: window.sceneEditor.replay(replay),
        }), replay);
        assert.deepEqual(initial.bytes, Array.from(bytes));
        assert.equal(initial.assertions, 9);
        const baseline = await page.evaluate(() => window.sceneEditor.telemetry.snapshot());
        assert(baseline.operations.every(row => !row.enabled && !row.accepted && !row.rejected));
        await page.evaluate(() => window.sceneEditor.telemetry.configure('compile-text,compile-typed,reload'));
        const originalElement = await page.locator('#bucket').elementHandle();
        const originalText = await page.locator('#text').inputValue();
        const changedText = originalText.replaceAll(':510', ':240');
        await page.locator('#text').fill(changedText);
        await page.waitForFunction(() => window.sceneEditor.inspect().presented === 2);
        assert.equal(await page.evaluate(() => window.sceneEditor.inspect().value), 240);
        const typed = JSON.parse(await page.locator('#typed').inputValue());
        typed[2][0][3][0][1] = 365;
        await page.locator('#typed').fill(JSON.stringify(typed));
        await page.waitForFunction(() => window.sceneEditor.inspect().presented === 3);
        assert.equal(await page.evaluate(() => window.sceneEditor.inspect().value), 365);
        const frames = await page.evaluate(() => window.sceneEditor.inspect().frames);
        await page.locator('#text').fill('v2 invalid');
        await page.waitForFunction(() => document.querySelector('#status').dataset.error === 'true');
        const active = await page.evaluate(() => window.sceneEditor.inspect());
        assert.equal(active.presented, 3);
        assert.equal(active.value, 365);
        assert(active.frames > frames);
        assert(await originalElement.evaluate(node => node === document.querySelector('#bucket')));
        const stats = await page.evaluate(() => window.sceneEditor.telemetry.snapshot());
        const operation = name => stats.operations.find(row => row.feature === name);
        assert.equal(operation('compile-text').accepted, 1);
        assert.equal(operation('compile-text').rejected, 1);
        assert.equal(operation('compile-typed').accepted, 1);
        assert.equal(operation('reload').accepted, 2);
        assert.equal(operation('reload').rejected, 1);
        assert.equal(operation('reload').observed_latency.reduce((a, b) => a + b, 0), 3);
        assert.equal(operation('sample').accepted, 0);
        console.log('Browser passed 9 native-identical replay assertions, both live edit routes, and failed-edit rollback.');
    } finally {
        await browser?.close();
        server.close();
    }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
