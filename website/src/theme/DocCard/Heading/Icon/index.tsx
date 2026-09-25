// Swizzled (ejected) from @docusaurus/theme-classic to draw nothing.
//
// The stock card prefixes every title with an emoji — 🗃 for a category, 📄
// for a page, 🔗 for a link — on every generated index and every
// `DocCardList`. The site is otherwise type and rule lines only, so the
// override is here, at the one leaf that renders the glyph, rather than on
// each page that happens to list cards. `DocCard/Heading` still passes an
// `icon`; returning null is the whole change.

import type {ReactNode} from 'react';
import type {Props} from '@theme/DocCard/Heading/Icon';

export default function DocCardHeadingIcon(_props: Props): ReactNode {
  return null;
}
