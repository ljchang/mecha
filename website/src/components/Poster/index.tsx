// A concept-schematic poster, on the page.
//
// Every drawing is shown as its *plate* — the sheet on its cream paper — in
// both themes. Cutouts with the paper removed were tried and dropped: the
// linework is black ink, so on the dark ground a cutout loses its wordmark,
// callouts and panel edges, and on the light one it read as less finished
// than the sheet. A plate is an object on the page, so it gets a sheet's edge.
//
// Files live in `static/img/posters/`, re-encoded from the originals at
// 1536×1024 — about twice the widest column a docs page gives them.

import type {ReactNode} from 'react';
import clsx from 'clsx';
import useBaseUrl from '@docusaurus/useBaseUrl';

import styles from './styles.module.css';

type Props = {
  /** Basename in `static/img/posters/`, without the `-plate.webp` suffix. */
  name: string;
  alt: string;
  caption?: ReactNode;
  /** Above the fold: fetch it first rather than when it scrolls into view. */
  eager?: boolean;
  className?: string;
};

export default function Poster({
  name,
  alt,
  caption,
  eager = false,
  className,
}: Props): ReactNode {
  const src = useBaseUrl(`/img/posters/${name}-plate.webp`);
  return (
    <figure className={clsx(styles.poster, className)}>
      <img
        src={src}
        alt={alt}
        width={1536}
        height={1024}
        loading={eager ? 'eager' : 'lazy'}
        fetchPriority={eager ? 'high' : undefined}
        decoding="async"
      />
      {caption && <figcaption>{caption}</figcaption>}
    </figure>
  );
}
