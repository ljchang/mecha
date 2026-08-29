// The live web surface, framed.
//
// The page inside this iframe is the *real* app — `web/src/`, built by the
// same Vite config into `static/demo/` at build time (see
// `scripts/build-demo.mjs`). Nothing here draws a control or picks a colour.
// If the surface looks a certain way on this page, that is how it looks on
// your phone. What differs is only what comes back over `/api`: fixtures
// instead of a box, invented by hand, because this repository is public and
// the real thing is a view onto one person's mail (`web/src/demo/fixtures.js`
// says so at length).
//
// Three things this needs that `GalleryFrame` did not:
//
//   - **A fixed height, not a measured one.** A gallery form is a document and
//     wants to be as tall as its content. This is an *app*: its shell is
//     `height: 100dvh` and it lays itself out inside whatever box it is given,
//     so measuring `scrollHeight` would ask a question with no answer.
//   - **No theme stamping.** The mecha surface is dark-only by design — one
//     accent hue on the void ground, hazard amber reserved as a signal. There
//     is no light variant to switch it to, so the frame does not pretend
//     otherwise; it sits in its own bezel and reads as a device in the page.
//   - **Page switching without a reload.** `App.svelte` routes on the hash, so
//     moving between pages is `contentWindow.location.hash`, which keeps the
//     app's state and its scripted conversation alive. Re-setting `src` would
//     reboot the app on every tab.

import React, {useEffect, useRef, useState} from 'react';
import useBaseUrl from '@docusaurus/useBaseUrl';
import BrowserOnly from '@docusaurus/BrowserOnly';

import styles from './styles.module.css';

type Props = {
  /** Hash route to open, e.g. `chat` or `review/frontdoor`. `home` is the default. */
  page?: string;
  /** Tabs to offer above the frame. Omit for a single fixed page. */
  pages?: {hash: string; label: string}[];
  /** What this is, under the frame. */
  caption?: React.ReactNode;
  /** Frame height in pixels. */
  height?: number;
};

function Frame({page = 'home', pages, caption, height = 700}: Props) {
  const base = useBaseUrl('/demo/');
  const [current, setCurrent] = useState(page);
  const [missing, setMissing] = useState(false);
  const frame = useRef<HTMLIFrameElement>(null);
  const booted = useRef(false);

  useEffect(() => {
    let live = true;
    // HEAD rather than GET: this only needs to know the build is there, and
    // the bundle is 180 kB that the iframe is about to fetch anyway.
    fetch(`${base}index.html`, {method: 'HEAD'})
      .then((response) => {
        if (live && !response.ok) setMissing(true);
      })
      .catch(() => {
        if (live) setMissing(true);
      });
    return () => {
      live = false;
    };
  }, [base]);

  // Same-origin, so the hash can be set directly. Only after first load —
  // before that the `src` carries it, and writing to a document that has not
  // arrived yet does nothing.
  useEffect(() => {
    if (!booted.current) return;
    const window_ = frame.current?.contentWindow;
    if (window_) window_.location.hash = current === 'home' ? '' : current;
  }, [current]);

  if (missing) {
    return (
      <div className={styles.absent}>
        <p>
          <strong>The live demo is not built.</strong> It is the app in{' '}
          <code>web/</code> compiled against fixtures, and the docs build makes it. Run{' '}
          <code>npm run build-demo</code> here, or <code>npm run build:demo</code> in{' '}
          <code>web/</code>.
        </p>
      </div>
    );
  }

  const source = `${base}index.html${current === 'home' ? '' : `#${current}`}`;

  return (
    <figure className={styles.figure}>
      {pages && (
        <div className={styles.bar}>
          <div className={styles.tabs} role="group" aria-label="Page">
            {pages.map((p) => (
              <button
                key={p.hash}
                type="button"
                aria-pressed={p.hash === current}
                className={p.hash === current ? styles.on : styles.off}
                onClick={() => setCurrent(p.hash)}>
                {p.label}
              </button>
            ))}
          </div>
          <a className={styles.open} href={source} target="_blank" rel="noreferrer">
            open on its own ↗
          </a>
        </div>
      )}
      <div className={styles.stage}>
        <div className={styles.device} style={{height}}>
          <iframe
            ref={frame}
            src={source}
            title="the mecha web surface"
            loading="lazy"
            className={styles.frame}
            onLoad={() => {
              booted.current = true;
            }}
          />
        </div>
      </div>
      <figcaption className={styles.caption}>
        {caption}{' '}
        <span className={styles.disclaimer}>
          This is the real app with invented data behind it — every name and message is
          fiction, and the reply is scripted rather than a model’s.
        </span>
      </figcaption>
    </figure>
  );
}

// `BrowserOnly` for the same reason `GalleryFrame` uses it: rendering the
// iframe server-side would put one in the static HTML that gets a second life
// on hydration, booting the app twice.
export default function WebFrame(props: Props): React.JSX.Element {
  return (
    <BrowserOnly fallback={<div className={styles.placeholder} />}>
      {() => <Frame {...props} />}
    </BrowserOnly>
  );
}
