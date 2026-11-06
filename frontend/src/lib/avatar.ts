// The person's avatar (card #393): a picture they pick, shrunk here to a
// small square so it fits on their member.profile in every space they
// share (coord::person_avatar::MAX_LEN caps it at 32 KB).

// AvatarSize is the square's side in pixels: sharp at the largest face
// the window draws (64px) on a 2x screen.
export const AvatarSize = 128

// MaxAvatar mirrors coord::person_avatar::MAX_LEN.
export const MaxAvatar = 32 * 1024

// cropSquare is the centred square of a w×h picture: its left, top and side.
export function cropSquare(
  w: number,
  h: number
): { x: number; y: number; side: number } {
  const side = Math.min(w, h)
  return { x: Math.floor((w - side) / 2), y: Math.floor((h - side) / 2), side }
}

// isAvatar mirrors coord::person_avatar::valid for a non-empty one: a PNG, JPEG
// or WebP data URL of plain base64, within the cap. Anything else is drawn
// as initials instead.
export function isAvatar(a: string | undefined): a is string {
  return (
    !!a &&
    a.length <= MaxAvatar &&
    /^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+/=]+$/.test(a)
  )
}

// initials is a face's letter when there is no picture: the name's first
// letter, uppercased ("?" for none).
export function initials(name: string): string {
  const ch = [...name.trim()][0]
  return ch ? ch.toUpperCase() : "?"
}

// shrinkAvatar turns a picked file into the avatar: its centred square at
// AvatarSize, as WebP (JPEG where WebP can't be written), lowering the
// quality until it fits.
export async function shrinkAvatar(file: Blob): Promise<string> {
  const bmp = await createImageBitmap(file).catch(() => {
    throw new Error("That file isn't a picture Irori can read.")
  })
  const { x, y, side } = cropSquare(bmp.width, bmp.height)
  const canvas = document.createElement("canvas")
  canvas.width = AvatarSize
  canvas.height = AvatarSize
  const g = canvas.getContext("2d")
  if (!g) throw new Error("Can't draw the picture here.")
  g.imageSmoothingQuality = "high"
  g.drawImage(bmp, x, y, side, side, 0, 0, AvatarSize, AvatarSize)
  bmp.close()
  for (const q of [0.9, 0.8, 0.65, 0.5]) {
    for (const type of ["image/webp", "image/jpeg"]) {
      const url = canvas.toDataURL(type, q)
      if (url.startsWith(`data:${type};`) && isAvatar(url)) return url
    }
  }
  throw new Error("That picture is still too big after shrinking it.")
}
