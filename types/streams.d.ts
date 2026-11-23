import { Readable, Writable } from "stream"

/* Types JSDoc cannot write in a way jsdoc (npm run docs) also reads. */

/** The Readable a channel's createReadStream returns, which says what it carries. */
export type channelreadstream = Readable & {
  format: string
  samplerate: number
  numchannels: number
  direction: string
  readerId: number
}

/** The Writable a channel's createWriteStream returns, which says what it takes. */
export type channelwritestream = Writable & {
  format: string
  samplerate: number
  numchannels: number
  writerId: number
}
