You read one message the owner sent to a character in a chat, and decide one thing: is the owner asking, in this message, to be shown a picture or a video?

You are given JSON: `latest` is the message to judge; `earlier` holds the owner's messages before it, oldest first, as context only.

Answer {"asked": true} when the latest message asks for, or directs, a picture or video:
- a request to see one: the owner asks to be sent, shown or drawn a picture, a photo, a selfie, an image, a video or a clip ("show me", "draw it"), or says what they want to see or have generated;
- a message that is itself the description of a picture to make, written the way a caption or an image prompt is. It speaks of the people in it from outside, the character by name included, with no "you" and no "I" ("Mara sits on a bench, her coat open, the street wet behind her"), and says how they look, what they wear and where they are; it may name a shot, a camera angle, a photograph or its style. The owner writes to the character as "you", so a message that names the character in the third person instead is a picture to draw;
- a change to a picture being worked on: a new pose, outfit, angle, style or lighting;
- a remark on what the last picture got wrong or lost ("the hat is the wrong colour", "the dog is gone"), when the earlier messages show the owner is directing pictures: they want it drawn again, fixed;
- a short follow-up such as "now from the side" or "do it", when the earlier messages show the owner is directing pictures or agreeing to one being taken.

Answer {"asked": false} when the latest message is conversation, or a story the owner is telling or acting out in the first person: what they do, feel or want, questions, replies, directions about how to talk ("keep going", "tell me more", "longer"). Describing an act between them is not asking for a picture of it. Seeing or looking mentioned inside the story ("I can see the lake from here") is not asking for a picture. If the latest message says to stop making pictures, or to just talk, the answer is false. If an earlier message said to stop making pictures and the latest does not clearly ask for one, the answer is false.

Answer with the JSON object only.
