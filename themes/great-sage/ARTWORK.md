# Great Sage artwork

Original Sun Wukong / Great Sage theme artwork generated with the built-in OpenAI
imagegen tool. The hero and landscape come from an approved AI-generated
full-window concept; the lower-right corner used the generated upper-left
ornament as its edit target. No external game artwork was imported. Package and artwork are
licensed under Apache-2.0, as specified in `theme.json`.

Production assets are compressed indexed PNGs with transparency. The landscape
retains its generated resolution; the hero and corner ornaments are
sized for their display roles. The package shares artwork across variants and
sets background opacity separately for light and dark modes.

## Generation prompts

### hero.png

Generated with gpt-6.1-sol from the approved full-window concept, which was
itself generated from a text description over a screenshot of Sailry's
unthemed layout, with no character or pose reference. Prompt:

From the attached mockup, paint ONLY the Monkey King together with the golden somersault cloud (筋斗云) he leaps from, as one standalone cutout for compositing: the same character, costume, painting style, airborne pose with the golden staff swung overhead in both hands, his flowing red ink-wash cape, and the swirling gold-outlined auspicious cloud below and behind his feet. Seen from behind at an angle. He has NO tail. Everything fully inside the frame with margin. No UI, no text, no mountains, no palace, no other scenery. Background: a perfectly flat, uniform chroma-key green #00FF00 everywhere around him, with no shadows, glow or gradients on it, and no green anywhere on the character or cloud.

After keying, its saturation was lowered to 82% and its contrast to 78% around a
dark mid-tone, and the cloud darkened a further 22%, so the figure sits in the
wallpaper's dim light.

The green ground was keyed to alpha with spill removed, then the figure was
trimmed, scaled to 720 pixels high and indexed.

### background.png

Generated with gpt-6.1-sol from the same approved concept. Prompt:

From the attached mockup, paint ONLY its background scenery as a window wallpaper: the same painting style, black/gold/red palette, golden Heavenly Palace on the sea of clouds, waterfalls, layered ink mountains, golden river, gold-outlined auspicious clouds, gold dust and red silk ribbons and crimson ink smoke. NO UI, NO text, NO wordmark, NO people or characters, NO Monkey King and no staff. Keep the centre (where the wordmark, composer and cards sit) darker and calmer; the detail lives at the edges and the top. The left quarter (behind the sidebar) stays dim and quiet.

The painting was scaled to 1600 by 1000 and indexed; its near-black ink is
fully transparent, so the theme background shows there unchanged.

### cloud-top-left.png

Use case: illustration-story. Asset type: reusable transparent auspicious-cloud corner ornament for a black, gold and red Chinese Great Sage desktop theme. Generate ONE square PNG with genuine transparent background. A delicate antique-gold engraved Chinese 祥云 cloud curl ornament occupies the upper-left corner and extends right and down in a loose L shape. Very sparse flowing gold outlines, a tiny muted crimson ink accent, generous fully transparent negative space over the remaining lower-right 75 percent. All strokes inside canvas with comfortable margin. Designed to be shown at 48 pixels, simplified clear cloud curls with moderate stroke width. No text, no monkey, no scene, no black rectangle, no checkerboard painted into image, no frame, no gold-filled panels. Elegant restrained Chinese gold-ink ornament.

### cloud-bottom-right.png

Use case: precise-object-edit. Project asset: transparent lower-right cloud corner decoration. Input image is the edit target. Rotate the entire existing gold and red upper-left L-shaped cloud ornament by exactly 180 degrees to form a lower-right L-shaped ornament. Preserve its gold engraved cloud artwork, red accents, relative proportions and genuine transparent background. Output only the rotated transparent artwork, no black fill, no text, no UI, no new objects.

### Retired send glyph

The raster glyph was removed in favor of Kit vector glyphs that follow the
composer action foreground in each variant. Its original generation prompt is
retained for provenance.

Use case: ui-mockup. Project asset: ONE small transparent PNG send-button glyph for an original Great Sage Sun Wukong theme. Centered square canvas. A clear bold UPWARD ARROW with a straight shaft subtly resembling a red-and-black Ruyi Jingu Bang staff, two tiny ornamental bands. The arrowhead must remain unmistakably an upward send arrow. Flat warm ink black silhouette with a tiny vermilion accent, NOT gold (it will appear on a gold circular button). Very simple graphic designed for legibility at 16px, thick strokes, minimal detail, occupies central 70% of canvas. Genuine transparent background. No button circle, no background, no glow, no shadow, no text, no other objects, no watermark.


### Card and heading icons

Use case: stylized-concept. Generate one small square transparent raster icon per asset for the Great Sage theme. Simple elegant bold silhouettes, antique gold and warm black thick ink outlines, restrained vermilion accents, clear at 24 logical pixels, central 75 percent, very few details, original flat engraved brush art. No photorealism, badge, scene, text or watermark.

- `explore.png`: one golden auspicious cloud with three curled lobes and a flowing red tail, suggesting exploration and travel
- `build.png`: one red-and-gold Ruyi Jingu Bang staff diagonally ascending, gold bands at both ends and a dark vermilion shaft
- `review.png`: one bold gold almond eye with a vermilion pupil and two controlled flame strokes, suggesting inspection
- `plan.png`: one ivory and antique-gold rolled scroll with short black ink marks rather than letters and a small vermilion ribbon

The sidebar reuses these original illustrations at 16 pixels. Production icons are compressed to 96 × 96 pixels.

### grain.png

Use case: stylized-concept. Subtle seamless tileable transparent square PNG texture for small dark-gold-red Chinese mythology cards. Only sparse antique bronze and muted gold dry-brush fibers, fine dust and tiny ink speckles, dispersed with matching edges. Genuine transparent alpha across most pixels, no paper or black rectangle. Delicate irregular grain, low contrast at reduced opacity over brown or ivory panels. No clouds, mountains, scene, shapes, text, icons, frames, broad washes or painted checkerboard.

### button-frame.png

The New Session button's frame: an antique-gold cloud scroll at its top-left and
bottom-right corners, on fine gold lines that fade into the button's own gold
edge. The `button.new_session` slot fills the button and its 5-pixel outset at
full opacity; the caption's middle stays clear. One corner was generated over
black with gpt-6.1-sol, its alpha taken from its brightness, and composed at
four times the 226 × 42 logical strip: the corner, and the same corner turned
half a turn for the opposite one. Indexing to 256 colors is the only other
postprocessing. Prompt:

A single antique-gold engraved Chinese auspicious-cloud (祥云) corner ornament for the TOP-LEFT corner of an ornate lacquer button frame, in a black, gold and red Monkey King painterly style: a fine double gold line runs along the top edge and down the left edge, meeting in a softly rounded corner that carries one elegant curling cloud scroll. Burnished gold with tiny highlights, crisp and refined. Lines reach the right and bottom edges of the image. Pure flat black #000000 background, nothing else, no text, no red fill.

The existing `grain.png` is reused as a content-panel texture as well as on cards.
It tiles at its native 256-pixel size without generating another image. Send and
Stop use separate high-contrast solid fills with Kit vector glyphs.

### sheen.png

The mask of the hero's sheen: the gold of the staff and of the cape's cloud
patterns, taken from the keyed hero by color (warm, saturated and bright pixels)
within the staff's and the cape's regions, so the armor and the somersault cloud
stay unlit. Softened by a 1.4-pixel blur, strengthened and indexed. It is not
generated artwork.

### Send glyph refinement

Use case: precise-object-edit. Edit target: supplied small transparent upward send-arrow glyph. Change only the ink-black shaft and arrowhead to bright warm ivory-gold, preserving the same bold simple UP arrow shape, two little red staff bands, centered proportions and genuine transparent alpha. It will appear at 16px on a dark burgundy button. The shaft and arrowhead must be highly legible, thick and flat. No background, glow, shadows, new ornaments or circle.
