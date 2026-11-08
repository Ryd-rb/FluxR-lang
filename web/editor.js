import init, {
    Scene, configureTelemetry, recordTelemetryReload, telemetryReloadEnabled, telemetrySnapshot,
} from './pkg/fluxr.js';

const fields = { text: document.querySelector('#text'), typed: document.querySelector('#typed') };
const status = document.querySelector('#status');
const bucket = document.querySelector('#bucket');
const rope = document.querySelector('#rope');
let active;
let activeTrack;
let revision = 0;
let presented = 0;
let frames = 0;
let timer;

function edit(format, source) {
    const timing = telemetryReloadEnabled() ? performance.now() : null;
    const accepted = apply(format, source);
    if (timing !== null) {
        const elapsedUs = Math.min(0xffffffff, Math.round((performance.now() - timing) * 1000));
        recordTelemetryReload(accepted, elapsedUs);
    }
    return accepted;
}

function apply(format, source) {
    let next;
    let nextTrack;
    try {
        if (format !== 'text' && format !== 'typed') throw new Error('Expected text or typed');
        next = Scene.compile(source, format === 'typed');
        const typed = next.structured();
        const height = JSON.parse(typed)[2].find(([object, field]) => object === 41 && field === 1);
        if (!height) throw new Error('Preview requires bucket channel 41.1');
        if (height[3].some(([, value]) => value < -1120 || value > 510)) {
            throw new Error('Well height outside -1120..=510 mm');
        }
        nextTrack = next.bind_track(41, 1);
        fields.text.value = next.text();
        fields.typed.value = typed;
    } catch (error) {
        nextTrack?.free();
        next?.free();
        status.dataset.error = 'true';
        status.textContent = `Revision ${revision} running · ${error.message}`;
        return false;
    }
    activeTrack?.free();
    active?.free();
    active = next;
    activeTrack = nextTrack;
    revision += 1;
    status.dataset.error = 'false';
    status.textContent = `Revision ${revision} · ${format} applied`;
    return true;
}

const render = (time) => {
    const height = activeTrack.sample(Math.floor(time));
    const y = 168 - height / 11;
    bucket.setAttribute('transform', `translate(220 ${y})`);
    rope.setAttribute('d', `M220 76V${y - 32}`);
    presented = revision;
    frames += 1;
    requestAnimationFrame(render);
};

try {
    await init();
    const response = await fetch('./well.fluxr');
    if (!response.ok || !edit('text', await response.text())) throw new Error('Example scene failed');
    requestAnimationFrame(render);
    for (const [format, input] of Object.entries(fields)) {
        input.addEventListener('input', () => {
            clearTimeout(timer);
            timer = setTimeout(() => edit(format, input.value), 180);
        });
    }
    window.sceneEditor = {
        edit,
        inspect: (time = 0) => ({ revision, presented, frames, value: activeTrack.sample(time) }),
        encode: () => Array.from(active.encode()),
        replay: (assertions) => active.replay(assertions),
        telemetry: {
            configure: configureTelemetry,
            snapshot: () => JSON.parse(telemetrySnapshot()),
        },
    };
} catch (error) {
    status.dataset.error = 'true';
    status.textContent = error.message;
}
