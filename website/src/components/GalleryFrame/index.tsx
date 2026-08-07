// A live form from the component gallery, framed.
//
// The pages inside these iframes are generated in mecha-factory by the code
// that serves the real forms, and copied into `static/factory/gallery/` at
// build time (see `scripts/sync-gallery.mjs`). Nothing here draws a control or
// picks a colour: if a form looks a certain way on this page, that is how it
// looks when it is served.
//
// Three things this component has to get right, none of which an <iframe> does
// on its own:
//
//   - **Follow the docs theme.** A form picks its scheme from
//     `prefers-color-scheme`, which an iframe reads from the operating system
//     rather than from the page around it. So a reader on a dark desktop who
//     toggles these docs to light gets a dark form in a light page. The frame
//     is same-origin, so the fix is to stamp `data-theme` on its document —
//     which is exactly the override every generated stylesheet declares.
//   - **Be as tall as its content.** A fixed height means a scrollbar inside a
//     scrollbar, and a form is the one thing a reader wants to see whole.
//   - **Offer the palettes the gallery actually has**, read from its
//     `index.json`, so a theme added in mecha-factory shows up here without
//     anyone editing this file.

import React, {useEffect, useMemo, useRef, useState} from 'react';
import useBaseUrl from '@docusaurus/useBaseUrl';
import {useColorMode} from '@docusaurus/theme-common';
import BrowserOnly from '@docusaurus/BrowserOnly';

import styles from './styles.module.css';

type ThemeInfo = {name: string; description: string};

type Props = {
  /** The page's file name inside a theme directory, e.g. `kinds.errors.html`. */
  page: string;
  /** What this rendering is, shown above the frame. */
  caption?: string;
  /** Height before the frame reports its own, in pixels. */
  minHeight?: number;
};

// Only ever used when `index.json` cannot be read — an offline `npm start`
// with no mecha-factory checkout, mostly. Deliberately the default palette
// alone rather than a guess at the full set: a stale list that looks complete
// is worse than a short one that obviously is not.
const FALLBACK: ThemeInfo[] = [{name: 'nocturne', description: 'The mecha palette.'}];

function Frame({page, caption, minHeight = 420}: Props) {
  const base = useBaseUrl('/factory/gallery/');
  const {colorMode} = useColorMode();
  const [themes, setThemes] = useState<ThemeInfo[] | null>(null);
  const [selected, setSelected] = useState<string>(FALLBACK[0].name);
  const [height, setHeight] = useState(minHeight);
  const [missing, setMissing] = useState(false);
  const frame = useRef<HTMLIFrameElement>(null);

  useEffect(() => {
    let live = true;
    fetch(`${base}index.json`)
      .then((response) => (response.ok ? response.json() : Promise.reject(response.status)))
      .then((contents) => {
        if (!live || !Array.isArray(contents?.themes) || contents.themes.length === 0) return;
        setThemes(contents.themes);
        setSelected(contents.themes[0].name);
      })
      .catch(() => {
        if (live) setMissing(true);
      });
    return () => {
      live = false;
    };
  }, [base]);

  // Re-applied on every load and on every toggle, because a reload resets the
  // document this wrote to.
  const dress = useMemo(
    () => () => {
      const document_ = frame.current?.contentDocument;
      if (!document_) return;
      document_.documentElement.dataset.theme = colorMode;
      const measure = () =>
        setHeight(Math.max(minHeight, document_.documentElement.scrollHeight));
      measure();
      // The form's own script shows and hides conditional fields as the reader
      // answers, so the height is not a property of the page as loaded.
      if (typeof ResizeObserver !== 'undefined' && document_.body) {
        const observer = new ResizeObserver(measure);
        observer.observe(document_.body);
        return () => observer.disconnect();
      }
      return undefined;
    },
    [colorMode, minHeight],
  );

  useEffect(() => dress(), [dress, selected]);

  if (missing) {
    return (
      <div className={styles.absent}>
        <p>
          <strong>The gallery is not built.</strong> It is generated in{' '}
          <a href="https://github.com/ljchang/mecha-factory">mecha-factory</a> and copied in
          at build time. Clone that repository beside this one, or run{' '}
          <code>npm run sync-gallery</code>.
        </p>
      </div>
    );
  }

  const available = themes ?? FALLBACK;
  const current = available.find((t) => t.name === selected) ?? available[0];
  const source = `${base}${current.name}/${page}`;

  return (
    <figure className={styles.figure}>
      <div className={styles.bar}>
        <div className={styles.themes} role="group" aria-label="Palette">
          {available.map((theme) => (
            <button
              key={theme.name}
              type="button"
              title={theme.description}
              aria-pressed={theme.name === current.name}
              className={theme.name === current.name ? styles.on : styles.off}
              onClick={() => setSelected(theme.name)}>
              {theme.name}
            </button>
          ))}
        </div>
        <a className={styles.open} href={source} target="_blank" rel="noreferrer">
          open on its own ↗
        </a>
      </div>
      <iframe
        ref={frame}
        key={`${current.name}/${page}`}
        src={source}
        title={caption ?? page}
        loading="lazy"
        className={styles.frame}
        style={{height}}
        onLoad={() => dress()}
      />
      {caption && <figcaption className={styles.caption}>{caption}</figcaption>}
    </figure>
  );
}

// `useColorMode` needs the browser, and so does everything else here. Rendering
// the frame server-side would put an iframe in the static HTML that gets a
// second, differently-themed life on hydration.
export default function GalleryFrame(props: Props): React.JSX.Element {
  return (
    <BrowserOnly fallback={<div className={styles.placeholder} />}>
      {() => <Frame {...props} />}
    </BrowserOnly>
  );
}
