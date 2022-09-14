# ProjectRTP

An RTP node addon which offers functionality to process RTP data streams and mix it. All performance tasks are implemented in C++ and use boost::asio for IO completion ports for high concurrency.

ProjectRTP is designed to scale to multiple servers serving other signalling servers. RTP and signalling should be kept separate and this architecture allows that. It is provided with a proxy server and client to allow for remote nodes.

ProjectRTP is designed to provide all the functions required for a PBX, including channel bridging, recording, playback and all the functions required for IVR.

Features

* Channel mixing for 2+ channels
* CODEC transcoding (between supported CODECs listed below)
* Call recording (WAV)
  * Multiple recorders per channel
  * Start, pause and end based on power detection or on command
* WAV playback using sound soup descriptions to play complex sentences from parts
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

Only when we release version 1.0.0 will the public API be settled.

## Release process

1. Update package.json to the new version number (see Version numbers above)
2. npm test && stress testing
2. npm publish
3. docker buildx prune
4. docker buildx build --platform linux/amd64,linux/arm64 -t tinpotnick/projectrtp:<version> . --push
5. Git release
6. Github tag <version> 

<version> should simply the semantic version number. For late stage test builds this could be <version>betax.

## Docker

Public docker images for amd64 and arm64 available on [Docker Hub](https://hub.docker.com/repository/docker/tinpotnick/projectrtp).

## Tests

We now have 3 different sets of tests.

### `npm test`

All tests are run from node. From the root directory run `npm test`. These test all our interfaces and should test expected outputs. These tests use mocha.

The folder is separated out into interface, unit and mock. The mock folder contains mock objects/functions required for testing. Unit tests are to help test internal functions. Interface tests are used to guarantee a stable interface for a specific version number. If these tests require changing (other than bug fixing i.e. a material API change) then a major version update will happen.

### node stress/index.js

These are designed to create real world scenarios - opening and closing multiple channels and random times and at load. This is designed to test for unexpected behaviour. These test do not provide a pass/fail - but might crash or produce unexpected output on bad behaviour. These have concurrency/race condition tests in mind.

### C++

Some tests are provided in C++. They are built with the compiler flags `-fsanitize=address -fsanitize=leak` to catch buffer overruns and leaks.

To build the test executable run `make` from the src directory.

## Dependencies

* spandsp-devel
* libilbc
* boost
* gnutls
* libsrtp
* openssl (for now the node scripts use openssl to generate a self signed cert for DTLS)

libilbc is included as a git submodule and requires building. This needs pulling which also
has a submodule so (although this is handled inteh Dockerfile if you are building an image): 

```
git submodule update --init --recursive
```

### Fedora

After installing standard build tools - including g++.

```
dnf install ilbc-devel spandsp-devel boost gnutls libsrtp libsrtp-devel
```

### Ubuntu

Build ilbc from the sub module plus install other dependancies.

```bash

apt install libboost-dev libspandsp-dev gnutls-dev libsrtp2-dev cmake 

cd ilbc
cmake . -DCMAKE_INSTALL_LIBDIR=/lib -DCMAKE_INSTALL_INCLUDEDIR=/usr/include; cmake --build .; cmake --install .
cd ..
```

## Docker/Podman

Podman has some shortfalls. Remove and install docker.

```
docker build .
```

### Multi arch targets

Qemu emulators are needed

```
docker run -it --rm --privileged tonistiigi/binfmt --install all
```

Or in one shot and build it. This create a docker image and container buildkit. 
```
docker buildx create --name rtpbuilder --use --bootstrap --platform linux/amd64,linux/arm64
```

To clean up if you want to remove this buildx env:
```
docker buildx rm rtpbuilder
```

Then to build, and push to Docker hub (REMEMBER to update all references to version number including in Dockerfile)
```
docker buildx prune
docker buildx build --platform linux/amd64,linux/arm64 -t tinpotnick/projectrtp:2.2.10 . --push
```

NB: updating version, Docker file, this readme and package.json all need updating - is there a way of automating this?

### Runing

The enviroment variable HOST should contain the host address of the control server - its default is 127.0.0.1. The variable PORT contains the port number to communicate over - with a default of 9002.

The enviroment variable PA is takes the below form- where the address is passed back to the control server so it can be published in SDP - i.e. this allows for multiple RTP nodes all on different IP addresses. If you do not pass this in, then the default script will determin your public IP address by calling https://checkip.amazonaws.com to obtain your IP.

docker run --restart=always --net=host -d -it --env PA=127.0.0.1 --name=prtp projectrtp:latest

## Example scripts

The examples folder contains 2 scripts. The simplenode can be used where no special funtionality is required. The standard projectrtp docker image points to this - so starting a docker instance will run up as a node and attempt to connect to a central control server.

The advancednode.js is an example which contains pre and post processing - which can be used, for example, downloading recordings, performing text to speech or uploading recordings when it has completed.

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

Example:

```js

const prtp = require( "@babblevoice/projectrtp" ).projectrtp

/* This switches this server to a central 
server and requires nodes to connect to us
to provide worker nodes */
prtp.listen()

const codec = 9 /* G722 */

/* 
Now just open our channel 
The remote comes from your client (web browser?)
including options for WebRTC if it is a web browser.
*/
let channela = prtp.openchannel( {
      "remote": { address, port, codec }
      } )


let channelb = prtp.openchannel( {
      "remote": { "address": otheraddress, "port": otherport, codec }
      } )

/*
Offer both of these channels to the remote clients (convert to SDP?)
*/

/* Ask projectrtp to mix the audio of both channels */
channela.mix( channelb )

/* Keep calling the mix function with other channels to create a conference */
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
* Add support for cppcheck on commit and tidy up current warnings (see below).

```
cppcheck --enable=warning,performance,portability,style --error-exitcode=1 src/
```
