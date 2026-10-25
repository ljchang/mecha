# Prompting video_generate (MiniMax H3)

H3 renders brief clips, picture and sound together, and it learned from prompts laid out in a fixed structure. The tool assembles that structure; what you supply are the parts that differ between clips, shaped as described here.

## Left to the tool

- **Library characters.** Anyone from the library you mention, in `prompt` or in `scene`, is matched up with their portrait, the description stored for them and their voice.
- **Labels and sections.** The tool writes the sections H3 expects around your timeline (who each subject is, what of them to keep, a summary) and swaps each library name for its subject label. Spoken lines keep the names as you wrote them.
- **Personas.** When a persona asks, she appears in the clip unless the call gives other references.
- **Appearance.** How a library character looks (their face, hair, skin, build) is the portrait's and the description's job. Write what changes from clip to clip: outfit, pose, what they do, expression, where they stand in frame.

## Structured requests: `scene` (use this when you can)

Fill `scene` and the tool writes the H3 prompt for you: it numbers speakers, attaches voices, ends every line properly, keeps people who are listening quiet, and checks that the timing fits. `shots` is the only required part.

- `style`: a single concrete line covering look and mood. H3 responds to light, colour and pace rather than adjectives about feelings, so prefer `Live-action, cinematic, calm and wry, cool overcast light` to `a cosy vibe`.
- `setting`: a `description`, plus `light` if it matters.
- `cast`: an entry for each person:
  - `name`: their library name, or a few words describing someone not in the library;
  - **`wearing`, every time.** Nothing of a portrait's outfit survives, so without it H3 dresses them however it likes;
  - `doing`: their activity as the clip starts (optional);
  - `notes`: anything more about them (optional);
  - `pov`: `true` for the person the clip is seen through, in a first-person shot. Their face never appears, only their hands and body, and their voice is attached only if they have lines. Describe what their hands are doing in the action.
- `shots`: an entry for each shot:
  - `framing`, such as `A wide two-shot`;
  - `action`: what happens, as prose, in sequence;
  - `camera`: optional, such as `The camera holds still`;
  - `at`: the second the shot begins; omit it and the shots share the length evenly.
- `lines`: an entry for each spoken line, in the order spoken:
  - `who`;
  - `says`: only the words;
  - `delivery`: optional, the manner of speaking;
  - `shot`: optional, the shot it falls in. Without it, a line goes to the next shot whose `action` says someone speaks ("…looks up and speaks"), and to the first shot when no action does. Write "speaks" into the action at the moment each line falls.
- `soundscape`, `music`, `aspect` and `seconds` work as described further down.

Timing trouble, such as more words than the length allows or a cut after the end, comes back as notes beside the finished clip; it never blocks the render.

### A scene, written out

```
scene:
  style: Live-action, cinematic, calm and wry, cool overcast light
  setting: { description: A cluttered bicycle workshop, light: Grey light from a skylight }
  cast:
    - { name: Maya, wearing: a navy boiler suit with rolled sleeves, doing: kneels beside an upturned bike }
    - { name: Wren, wearing: a yellow raincoat }
  shots:
    - { framing: A wide two-shot, action: Wren shakes rain off her hood in the doorway, camera: The camera holds still }
    - { at: 3.25, framing: A low angle on the bench, action: Maya spins the front wheel and listens }
  lines:
    - { who: Maya, says: "Chain or brakes", delivery: without looking up }
    - { who: Wren, says: "Both, probably." }
  soundscape: Rain drumming on a skylight.
  seconds: 6
```

The tool turns that into a timeline that begins:

```
Live-action, cinematic, calm and wry, cool overcast light. [Shot 1] A wide two-shot. A cluttered bicycle workshop. Grey light from a skylight. Maya wears a navy boiler suit with rolled sleeves and kneels beside an upturned bike. Wren wears a yellow raincoat. Wren shakes rain off her hood in the doorway. The camera holds still. Maya (S1) says without looking up, <d>[English] Chain or brakes.</d> Wren listens with mouth closed.
```

## What H3 does reliably, and what it does not

Measured on this machine's 10 to 15 s clips (2026-10-25):

- **Expressions change only when the steps are named, in order.** "Her brows draw together, her eyes squeeze shut, her mouth falls open, her head tips back; then her eyes open and lock onto the camera" plays out within one shot. "Her expression changes" leaves the face still.
- **A push-in or zoom works on a face, not on a whole body.** On a close-up the camera moves in; on a shot showing someone head to knee, H3 moves their head and chest toward the lens instead, and the body comes out the wrong shape. To end close on a face, cut to a close-up (`at` a few seconds before the end) and push in there.
- **Change a pose at a cut.** Moving from one position to another mid-shot tangles limbs; cut, from the same angle, to the new position.
- **A face turned away stays away.** In a first-person shot from behind or above, the face hardly shows; give the lines their own close shot with the face turned to the camera.
- **What hands hold, they keep holding.** State each hand's grip, and say when it lets go if the action needs it free.
- **A described mark goes on whatever skin is in frame.** Say where it is, and when that part of the body is out of view but other skin is in it, say that skin is bare.
- **Clothes said only in words drift across a cut.** Repeat a distinctive garment's key features (colour, cut, neckline) in each shot that shows it.

## Writing `prompt` yourself

Give a timeline of roughly 120 to 400 words for a clip of 5 to 10 s, in the order things are seen.

1. **One opening line on the look**, for example `Live-action, cinematic, cool overcast light.`
2. **Then `[Shot 1]`**, covering, in this order: how it is framed (a wide shot, a medium shot, a two-shot or a close-up); each person's place in frame and outfit; the location and its light; and then the action in sequence, camera movement included.
3. **A cut is written** `[Shot 2] At 00:03.250, the shot cuts to …`, with a time that lies inside the clip. One or two shots suit 5 to 6 s; up to four suit 10 to 15 s. For a small change, move the camera rather than cut.
4. **Camera movement** goes inside a sentence: `The camera <movement>`, optionally followed by an amplitude (`small` or `large`) and a speed (`slow` or `fast`), each written out as `with <amplitude> amplitude` and `at <speed> speed`.
   - Movements H3 knows: pushing in or pulling out; zooming (in or out); panning or trucking (left or right); tilting or a pedestal move (up or down); holding static; tracking; an arc; POV; a slight shake.
5. **Speakers are numbered** in the order they first talk, and keep their number for the whole clip; anyone silent gets no number.
   - Written as `Maya (S1) says without looking up, <d>[English] Chain or brakes.</d>` and then `Wren (S2) says, <d>[English] Both, probably.</d>`.
   - Only the spoken words sit between `<d>` and `</d>`; how they are said ("quietly", "dryly", "breathless") sits outside.
   - Two at once: `Maya and Wren (S1,S2) say, <d>[English] Not again!</d>`
   - Narration: say the voice is heard off screen and that the speaker's mouth stays shut, as in `Maya (S1) is heard off screen, <d>[English] …</d>, her mouth staying shut.`
6. **Keep the speech inside the length:** roughly 2.5 words for each second of talking, leaving a beat of quiet at each end. Five seconds is room for a short line or two.
7. **Lettering in shot** (a sign, a label) goes in double quotes, spelled exactly.
8. **Describe what can be seen and heard:** "she wipes grease off her thumb onto her sleeve" rather than "a lovely moment".

### `soundscape`: optional, though worth giving

One to four sentences on the room tone and physical sounds of the clip: rain, steps, a ratchet clicking, a kettle, breathing, laughing. Leave out speech and music.

### `music`: optional

Up to three sentences on score heard by the audience alone, naming instruments, tempo and dynamics, with no feeling words. Omit it for no score. Music that people in the scene can hear, such as a radio, belongs in the prompt.

## Settings

- `seconds`: from 4 to 15, 5 by default. Rendering time grows quickly with length.
- `aspect`: 16:9, 4:3, 1:1, 3:4 or 9:16.
- `size`: `full`, H3's native 768p and the default, where faces stay stable as people move; or `draft`, about 480p, for a fast try, where small faces far from the camera may blur.
- `turbo`: on by default. Turn it off (`false`) only for a version worth keeping, once the owner likes one or asks for a final ("no turbo" means `false`): all 20 steps, giving visibly better faces and mouths that match the words, at roughly four times the wait, so mention the wait when you start one.
- **Expected render times**, measured on one GB10 over 59 clips (2026-10-16); more references add time: full size, 5 s: around 6 minutes with turbo and 20 to 30 without. Full size, 10 s: around 11 minutes with turbo and about an hour without. Draft, 5 s: around 2 minutes.
- `turbo`, `seconds`, `size` and `aspect` stay as this chat's previous clip had them; give one only to change it, and the result lists what carried over.
- Clips take turns: this machine renders one at a time, so a clip requested during another's render waits and then starts by itself. Stop cancels a waiting clip as well.
- `references`: needed only for a picture out of this chat's own files (`images/….png`) or for a library character the prompt does not mention.
- `first_frame` and `last_frame`: begin or end **exactly** on a picture out of this chat's files, to carry on from something: the final frame of the last clip, or a scene you have already drawn. Everyone in that picture is still matched to their portrait, description and voice, and the clip carries on from the frame as your timeline says. Make the opening of the timeline agree with the picture: who stands where, wearing what.
- `seed`: reproduce a clip exactly.

## A last check before calling

- Everyone is named, or described clearly when they are not library characters.
- Each speaker carries a number such as `(S1)`, each line sits in `<d>[English] …</d>` and ends in `.`, `?` or `!`, and the words fit the length.
- While one person talks, say that the others keep their mouths closed, for example "Wren listens with her mouth closed", so the wrong lips do not move.
- Each person's clothes are given. The portrait decides the face, never the outfit.
- Every shot time lies inside the clip.
- A soundscape is given.
