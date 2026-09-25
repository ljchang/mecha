// A concept-schematic poster, on the page.
//
// Each drawing ships as a *plate* — the sheet on its cream paper — and some
// also as a *cutout* with the paper removed. The cutout is only usable on a
// light ground: the linework is black ink, so on the dark theme the wordmark,
// the callouts and every panel edge disappear into the background. So the
// light theme shows the cutout where one exists and the dark theme always
// shows the plate, framed as a sheet pinned to the page. `ThemedImage` rather
// than a CSS swap for the same reason the hero logo uses it: two different
// files, not one file restyled.
//
// Files live in `static/img/posters/`, re-encoded from the originals at
// 1536×1024 — twice the widest column a docs page gives them.

import type {ReactNode} from 'react';
import clsx from 'clsx';
import useBaseUrl from '@docusaurus/useBaseUrl';
import ThemedImage from '@theme/ThemedImage';

import styles from './styles.module.css';

type Props = {
  /** Basename in `static/img/posters/`, without `-plate` / `-cutout`. */
  name: string;
  /** Whether a `-cutout` file exists for the light theme. */
  cutout?: boolean;
  alt: string;
  caption?: ReactNode;
  /** Above the fold: fetch it first rather than when it scrolls into view. */
  eager?: boolean;
  className?: string;
};

export default function Poster({
  name,
  cutout = false,
  alt,
  caption,
  eager = false,
  className,
}: Props): ReactNode {
  const plate = useBaseUrl(`/img/posters/${name}-plate.webp`);
  const light = useBaseUrl(`/img/posters/${name}-${cutout ? 'cutout' : 'plate'}.webp`);
  return (
    <figure
      className={clsx(styles.poster, !cutout && styles.plateOnly, className)}>
      <ThemedImage
        alt={alt}
        sources={{light, dark: plate}}
        width={1536}
        height={1024}
        loading={eager ? 'eager' : 'lazy'}
        decoding="async"
      />
      {caption && <figcaption>{caption}</figcaption>}
    </figure>
  );
}
