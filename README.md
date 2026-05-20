# ProjectRTP

Also see:
[MIGRATE.md](./MIGRATE.md)
[ARCHITECTURE.md](./ARCHITECTURE.md)


[![Build](https://github.com/babblevoice/projectrtp/actions/workflows/buildimage.yaml/badge.svg)](https://github.com/babblevoice/projectrtp/actions/workflows/buildimage.yaml)

An RTP node addon which offers functionality to process RTP data streams and mix it. All performance tasks are implemented in Rust (a napi-rs native module) with a per-channel tokio actor model for high concurrency.

ProjectRTP is designed to scale to multiple servers serving other signalling servers. RTP and signalling should be kept separate, and this architecture allows that. It is provided with a proxy server and client to allow for remote nodes.

ProjectRTP is designed to provide all the functions required for a PBX, including channel bridging, recording, playback and all the necessary functions for IVR.

Features

* Channel mixing for 2+ channels
* CODEC transcoding (between supported CODECs listed below)
* Call recording (WAV)
  * Multiple recorders per channel
  * Start, pause and end based on power detection or on command
* WAV playback using sound soup descriptions to play complex sentences from parts
* Combined play+record (playrecord) for zero-gap prompt-then-record with optional barge-in
* DTMF (RFC 2833) - send, receive and bridge
* DTLS SRTP (WebRTC)
* Highly scalable - server/node solution to scale out media nodes

## Version numbers

We use semantic version numbering.

Given a version number MAJOR.MINOR.PATCH, increment the:

1. MAJOR version when you make incompatible API changes,
2. MINOR version when you add functionality in a backwards compatible manner, and
3. PATCH version when you make backwards compatible bug fixes.

As part of this, we maintain jsdoc documentation to document our public interface to this programme.

## Release process

Releases are managed on GitHub actions. It builds and tests any version.

## Docker

Public docker images for amd64 and arm64 available on [Docker Hub](https://hub.docker.com/repository/docker/tinpotnick/projectrtp).

## Tests

Tests need the native module built (libilbc + libspandsp), so the simplest path is the Docker `test` stage. Locally, `npm run build` produces `build/Release/projectrtp.node` from the Rust crate and `npm test` runs the suite against it (requires `libilbc` and `libspandsp` on the host).

### Build the test image

```bash
docker build --target test -t projectrtp-test .
```

### Run all tests

```bash
docker run --rm projectrtp-test
```

The image's default command runs `mocha test/interface/*.js test/unit/*.js --exit`.

Note: the suite uses `test/interface/*.js` and `test/unit/*.js` — not `test/**/*.js` — because `test/basictests.js` and `test/codectests.js` are standalone scripts (not mocha tests) that will hang if loaded by mocha.

### Run a specific test

Most test files rely on the server test (`projectrtpserver.js`) to initialise the native module first, so include it when running individual files:

```bash
docker run --rm projectrtp-test \
  ./node_modules/mocha/bin/_mocha \
  test/interface/projectrtpserver.js test/interface/projectrtpplayrecord.js \
  --exit --timeout 20000
```

### Test with local edits

Mount `test/` and `lib/` so you can edit and re-run without rebuilding the native module:

```bash
docker run --rm \
  -v ./test:/usr/src/projectrtp/test \
  -v ./lib:/usr/src/projectrtp/lib \
  projectrtp-test \
  ./node_modules/mocha/bin/_mocha \
  test/interface/projectrtpserver.js test/interface/projectrtpplayrecord.js \
  --exit --timeout 20000
```

If you change Rust source files, rebuild the image.

### Rust unit tests

The crate's own unit tests run via cargo:

```bash
npm run rust:test          # or: cd rust && cargo test --lib
```

### Stress tests

Open and close channels at random times under load. Designed for concurrency and race condition testing — no pass/fail, but will crash or produce unexpected output on bad behaviour.

```bash
docker run --rm projectrtp-test sh -c \
  "cd /usr/src/projectrtp && npm run stress"
```

### Mic test tool

`test/tools/mictest.js` is a CLI tool for manually testing playrecord with a real microphone and speakers. It requires a local native build (`npm run build`) and `sox` on the host — see the Dockerfile for the full list of build dependencies.

```bash
node test/tools/mictest.js [--prompt <file>] [--output <file>] [--duration <ms>] [--interrupt] [--bargeinpower <n>]
```

Plays a prompt (or a generated test tone), records your voice, prints events in real time, and saves to `/tmp/mictest_recording.wav`.

## Example scripts

The examples folder contains two scripts which use this library. The simplenode can be used where no special functionality is required. The standard projectrtp docker image points to this - so starting a docker instance will run up as a node and attempt to connect to a central control server.

The advancednode.js is an example which contains pre and post-processing - which can be used, for example, downloading recordings, performing text to speech or uploading recordings when it has been completed.

### Running

The environment variable HOST should contain the host address of the control server - its default is 127.0.0.1. The variable PORT has the port number to communicate over - with a default of 9002.

The environment variable PA takes the below form- where the address is passed back to the control server so it can be published in SDP - i.e. this allows for multiple RTP nodes on different IP addresses. If you do not pass this in, the default script will determine your public IP address by calling https://checkip.amazonaws.com to obtain your IP.

docker run --restart=always --net=host -d -it --env PA=127.0.0.1 --name=prtp projectrtp:latest

## Codecs

ProjectRTP supports transcoding for the following CODECs

* PCMA
* PCMU
* G722
* iLBC

## Usage

2 example scripts can be found in the examples folder. The simplenode.js is used in the Docker container as it's main entry.

## Enviroment variables

These are parsed in the simplenode.js example.

* PORT - the port to connect to
* HOST - the host to connect to
* PA - the public address of this host - this is passed back over the control socket so it can be published to the client - if absent the script polls https://checkip.amazonaws.com for what looks like our public address.

## Little Example

This project contains all the functionality to use it as a local resource or as multiple nodes processing media. Switching between the 2 modes is seamless. When used in the node mode it has a simple protocol to communcate between server and nodes.

Each active node connects to the main server to offer its services to the main server. The main server then opens RTP channels (WebRTC or normal RTP) on any available. The protocol used can be viewed in the files in /lib.

### Server example

A server would typically run application logic before then opening media ports.

```js

const prtp = require( "@babblevoice/projectrtp" ).projectrtp

/* This switches this server to a central
server and requires nodes to connect to us
to provide worker nodes */
await prtp.server.listen()

const codec = 9 /* G722 */

/*
When you have a node configured and connected to this server...

Now open our channel:
The remote comes from your client (web browser?)
including options for WebRTC if it is a web browser.
*/
let channela = await prtp.openchannel( {
      "remote": { address, port, codec }
      } )


let channelb = await prtp.openchannel( {
      "remote": { "address": otheraddress, "port": otherport, codec }
      } )

/*
Offer both of these channels to the remote clients (convert to SDP?)
*/

/* Ask projectrtp to mix the audio of both channels */
channela.mix( channelb )

/* Keep calling the mix function with other channels to create a conference */
```


We could do it the other way round - i.e. instruct our control server to connect to nodes. We can either add multiple nodes or we can use a docker swarm to publish multiple nodes as one.
```js

const prtp = require( "@babblevoice/projectrtp" ).projectrtp

const port = 9002
const host = "192.168.0.100"

prtp.server.addnode( { host, port } )

/*
When we need a channel now, the library will
make that request to one of our nodes.
*/
let channela = await prtp.openchannel( {
      "remote": { address, port, codec }
      } )

```

### Node examples

Nodes are the work-horses of the network. The connect to a server and wait for instructions. See the examples folder for more examples, such as how to hook into events to perform functions such as downloading from storage such S3.

```js

const prtp = require( "@babblevoice/projectrtp" ).projectrtp

async function go() {

  prtp.run()

  prtp.node.listen( "0.0.0.0", 9002 )
  const pa = await wgets( "https://checkip.amazonaws.com" )

  prtp.setaddress( pa )

}

listen()

```

## Stats

When we receive an object back from our node (or if standlone just ourself), the object contains information about the
state of the sevrer. It includes items such as number of channels open, MOS quality score etc.

MOS is only included for our received measurements.

Tick time is the time taken to process RTP data before sending it on its way. We perform a small amount of work when we send and receive
RTP data but the majority of the work is performed on a tick - which gives an indication of load and capability.

Measurements are in uS (hence the us in the naming convention). In the example above the average ticktime was 84uS. A tick occurs on the ptime of a stream (which we only work with a ptime of 20mS).

If there are multiple channels being mixed they will all receive the same tick time as they are mixed in the same tick and this represents the total time for all channels.

### Play

Play a sound 'soup'. This is a list of files to play, potentially including start and stop times of the file to play. ProjectRTP Supports wav files. An example soup:

```js

channel.play( {
  "soup": {
    "loop": true,
    "files": [
      { "wav": "filename.wav", "start": 1000, "stop": 4000 }
    ]
  }
} )

```

* loop can either be true (continuous - well - INT_MAX) or a number
* loop can be in the main soup (i.e loop through all files) or on a file (i.e. loop through this file 3 times)
* The filename can be either in the param "wav", "pcma", "pcmu", "l168k", "l1616k", "ilbc", "g722". See note below on what they are used for, all contain the name of the file for that format (not completed).
* start - number of mS to start playback from (optional)
* stop - number of mS to stop playback (optional)

The start and stop param allows lots of snippets to be included in the same file.

The filename can be included in different param. If you supply the param, the RTP server will assume the file exists and the correct format. The goal with this param is to reduce CPU overhead of any transcoding when playing back a file.

#### Other soup examples

Queue announcement

```js
channel.play( {
  "loop": true,
  "files": [
    { "wav": "ringing.wav", "loop": 6 },
    { "wav": "youare.wav" },
    { "wav": "first.wav" },
    { "wav": "inline.wav" }
  ]
} )
```

We may also have combined some wav files:

```js
channel.play( {
  "loop": true,
  "files": [
    { "wav": "ringing.wav", "loop": 6 },
    { "wav": "youare.wav" },
    { "wav": "ordinals.wav", "start": 3000, "stop": 4500 },
    { "wav": "inline.wav" }
  ]
} )
```

### Record

Record to a file. ProjectRTP currently supports 2 channel PCM.

Can take options as per projectrtp:
file = <string> - filename to save as
uuid = <string> - channel uuid to record

In seconds up to MA max size (5 seconds?), default is 1 second
RMS power is calculated from each packet then averaged using a moving average filter.
poweraveragepackets = <int> moving average window to average power over

must have started for this to kick in - if left out will just start
startabovepower = <int>

When to finish - if it falls below this amount
finishbelowpower = <int>

used in conjunction with finishbelowpower - used in conjusnction with power thresholds
i.e. power below finishbelowpower before this number of mS has passed
minduration = < int > mSeconds

Must be above minduration and sets a limit on the size of recording
maxduration = < int > mSeconds

Must be 1 or 2
numchannels = < int > count

```js
channel.record( {
  "file": "/voicemail/greeting_1.wav",
  "maxduration": 10 * 1000 /* mS */,
  "numchannels": 1
} )
```

### Live audio — `channel.createReadStream`

Returns a standard Node `Readable` that emits decoded audio buffers as the channel receives / sends them. Use this to feed the audio into anything that takes a stream — STT, translation, captioning, WebSocket forwarders, on-disk capture — without going through the recorder / file path.

The byte shape of every emitted chunk is fixed for the lifetime of the reader and exposed as properties on the Readable:

```js
const reader = channel.createReadStream( {
  direction:   "in",    // "in" (default) | "out" | "both"
  format:      "l16",   // "l16" (default) | "pcma" | "pcmu" | "g722" | "ilbc"
  samplerate:  8000,    // 8000 (default) | 16000  — only meaningful for l16
  numchannels: 1        // 1 (default) | 2  — stereo interleaves L=in R=out
} )

reader.format        // "l16"
reader.samplerate    // 8000
reader.numchannels   // 1
reader.direction     // "in"
reader.readerId      // monotonic id (handy for log correlation)

reader.on( "data", ( buf ) => { /* Buffer, one 20 ms frame */ } )
reader.on( "end",  () => {} )   // fires on channel close or reader.destroy()

reader.pipe( wsStream )         // standard Node stream composition
reader.destroy()                // explicit teardown
```

**Direction**

- `"in"` — audio the peer sent us, post-decode. What you want for STT / voice biometrics on the caller.
- `"out"` — audio this channel is sending (player, echo, or the mix output). What the peer hears.
- `"both"` — stereo, interleaved, L = in, R = out. Requires `numchannels: 2`.

For independent per-speaker streams (e.g. diarised captions), create **two readers on the same channel** — one `direction: "in"`, one `direction: "out"`. Two mono streams, two independent STT pipelines, no coupling between them.

**Format**

`"l16"` gives linear PCM-16 LE at the chosen sample rate — the universal format that every STT / translation API accepts. The wire formats (`"pcma"`, `"pcmu"`, `"g722"`, `"ilbc"`) deliver raw codec bytes, useful when you want to forward a leg elsewhere without re-encoding.

**Backpressure and drops**

The pipeline never blocks the 20 ms RTP tick. If the consumer (or the Node event loop, or the napi queue) falls behind, frames are dropped. In practice this only bites if the consumer is genuinely stuck — typical STT / WebSocket consumers keep up with 20 ms frames comfortably.

**Example — pipe a leg into a WebSocket STT service:**

```js
const reader = channel.createReadStream( { direction: "in", format: "l16", samplerate: 16000 } )
const ws = new WebSocket( "wss://stt.example.com/stream?rate=16000&format=l16" )

reader.on( "data", ( buf ) => ws.send( buf ) )
ws.on( "message", ( transcript ) => console.log( "→", transcript.toString() ) )

/* When the call ends the reader emits `end` and the ws.send loop quiets. */
reader.on( "end", () => ws.close() )
```

**Example — dual-speaker call recording split per channel:**

```js
const caller = channel.createReadStream( { direction: "in" } )
const agent  = channel.createReadStream( { direction: "out" } )

caller.pipe( fs.createWriteStream( "/tmp/caller.pcm" ) )
agent.pipe(  fs.createWriteStream( "/tmp/agent.pcm" ) )
```

### Live audio — `channel.createWriteStream`

The symmetric counterpart to `createReadStream`. Returns a Node `Writable` that injects live audio into the channel's outbound leg — pipe in a TTS stream, a live translation feed, a bot's synthesised voice, anything that produces PCM.

```js
const writer = channel.createWriteStream( {
  format:      "l16",   // v1: l16 only
  samplerate:  8000,    // v1: 8000 only
  numchannels: 1        // v1: mono only
} )

writer.format        // "l16"
writer.samplerate    // 8000
writer.numchannels   // 1
writer.writerId      // monotonic id

writer.write( pcmBuffer )   // accepts any chunk size — framed internally
writer.end()                 // flushes the tail then reverts to silence
writer.destroy()             // immediate teardown, discards buffered samples

ttsStream.pipe( writer )     // standard Node stream composition
```

**Relationship to `play`**

A writer and `channel.play` share the single outbound-source slot on a channel. Starting a writer supersedes any active player (emits `play/end reason=new`); likewise `channel.play` supersedes an active writer. There is always at most one outbound source; when neither is present the channel sends silence.

**Framing**

You can `.write` any chunk size — the Rust side buffers bytes and frames them into 20 ms slots for the tick. An 8 kHz L16 stream needs ~16 000 bytes/sec (320 bytes per 20 ms frame), so a typical `.pipe` from a WebSocket or HTTP body keeps up comfortably.

**Backpressure**

The Rust-side buffer is bounded at 1 s (50 × 20 ms). If the consumer writes faster than the tick drains, `_write` delays its callback until space frees up — standard Node `Writable` backpressure kicks in and `.write` returns `false`. On underrun (nothing written for a tick) the channel sends silence for that slot.

**End-of-stream**

`writer.end()` drops the JS-side sender. The Rust `AudioWriter` sees the close, flushes any partial frame padded with silence, then retires itself from the channel. A `play/end reason=completed` event fires — symmetric with how a file-based `play` naturally ends.

**Example — pipe a TTS response into a live call:**

```js
const writer = channel.createWriteStream()

const res = await fetch( "https://tts.example.com/say", {
  method: "POST",
  body:   JSON.stringify( { text: "Your call is important to us." } ),
} )
// res.body is a Readable web stream of raw PCM-16 @ 8 kHz mono.
// Pipe straight through; backpressure is preserved end-to-end.
res.body.pipe( writer )
```

## Utils

### Tone generation

In order to make the RTP as scalable as possible, we will not support on the fly tone generation. Currently disk space is much cheaper than CPU resources. 1S of wav data sampled at 8K is 16K. Using soundsoup we can provide wav files for each supported codec and easily loop which requires very little CPU overhead.

What we need to provide is a utility to generate wav files which will generate tones for use in telecoms (i.e. ringing, DTMF etc).


```js

const prtp = require( "@babblevoice/projectrtp" ).projectrtp

let filename = "/some/file.wav"
prtp.tone.generate( "350+440*0.5:1000", filename )

```

The format attempts to closely follow the format in https://www.itu.int/dms_pub/itu-t/opb/sp/T-SP-E.180-2010-PDF-E.pdf - although that standard is not the clearest in some respects.

We currently support

* '+' adds 1 or more frequencies together
* '\*' reduces the amplitude (i.e. aptitude multiply by this value, 0.75 is 0.75 x max value)
* ':' values after the colon are cadences, i.e. duration before the tone moves onto the next tone
* '\~' start\~end, for example this can be used in frequency or amplitude, 350+440*0.5\~0:100 would create a tone starting at amp 0.5 reducing to 0 through the course of the 1000mS period

Examples

#### UK Dial tone:

```js
prtp.tone.generate( "350+440*0.5:1000" "dialtone.wav" )
```

#### UK Ringing

```js
prtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000" "ringing.wav" )
```

#### DTMF

||1209Hz|1336Hz|1477Hz|1633Hz|
|---|---|---|---|---|
|697Hz|1|2|3|A|
|770Hz|4|5|6|B|
|852Hz|7|8|9|C|
|941Hz|*|0|#|D|

Example, 1 would mix 1209Hz and 697Hz

```js
prtp.tone.generate( "697+1209*0.5:400" "dtmf1.wav" )
prtp.tone.generate( "697+1209*0.5/0/697+1336*0.5/0/697+1477*0.5/0:400/100", "dtmf1-3.wav" )
```


### TODO

* Format conversion between wav file types (l16, rate pcmu, pcma etc).
* Keep clippy clean on commit (enforced in CI):

```
cd rust && cargo clippy --all-targets -- -D warnings
```

# Ref

* [RFC 3550 - RTP: A Transport Protocol for Real-Time Applications](https://www.rfc-editor.org/rfc/rfc3550)
* [RFC 3711 - The Secure Real-time Transport Protocol (SRTP)](https://www.rfc-editor.org/rfc/rfc3711)
* [RFC 5763 - Framework for Establishing a Secure Real-time Transport Protocol (SRTP) Security Context Using Datagram Transport Layer Security (DTLS)](https://www.rfc-editor.org/rfc/rfc5763)
* [RFC 5764 - Datagram Transport Layer Security (DTLS) Extension to Establish Keys for the Secure Real-time Transport Protocol (SRTP)](https://www.rfc-editor.org/rfc/rfc5764)
* [RFC 8842 - Session Description Protocol (SDP) Offer/Answer Considerations for Datagram Transport Layer Security (DTLS) and Transport Layer Security (TLS)](https://www.rfc-editor.org/rfc/rfc8842)


