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

## Version numbers

We use semantic version numbering.

Given a version number MAJOR.MINOR.PATCH, increment the:

1. MAJOR version when you make incompatible API changes,
2. MINOR version when you add functionality in a backwards compatible manner, and
3. PATCH version when you make backwards compatible bug fixes.

As part of this, we maintain jsdoc documentation to document our public interface to this programme.

Only when we release version 1.0.0 will the public API be settled.

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

* ilbc-devel
* spandsp-devel
* boost
* gnutls
* libsrtp
* openssl (for now the node scripts use openssl to generate a self signed cert for DTLS)

### Fedora

After installing standard build tools - including g++.

```
dnf install ilbc-devel spandsp-devel boost gnutls libsrtp libsrtp-devel
```

## Node Module

node-gyp is required to build the module:

```
sudo npm install -g node-gyp
sudo npm install -g node-addon-api
```

From the src folder
```
node-gyp configure
node-gyp build
```

Or

```
node-gyp build --debug
```

## Docker/Podman

In the root is the script buildimage and Dockerfile. We pull from node latest and alpine Linux. The default image which can be used as a base or on its own.

podman run -it --rm --name=prtp --pod ourpod projectrtp:latest

## Codecs

ProjectRTP supports transcoding for the following CODECs

* PCMA
* PCMU
* G722
* iLBC

## Usage

2 example scripts can be found in the examples folder. The simplenode.js is used in the Docker container as it's main entry.

## Control

The server is managed via a control socket. It is designed so that multiple RTP servers can connect to a call control server to create a flexible voice switch.

The simplest way to spin up a projectrtp node is to use Docker.

### Enviroment variables

* PORT - the port to connect to
* HOST - the host to connect to
* PA - the public address of this host - this is passed back over the control socket so it can be published to the client - if absent the script polls https://checkip.amazonaws.com for what looks like our public address.

### Protocol

To be reviewed.

The control protocol is simple. A 5 byte header is sent:

```c++
class controlheader
{
public:
  char magik;
  uint16_t version;
  uint16_t length;
};
```

* magik must be 0x33
* version should currently be 0
* length indicates the number of bytes to follow

The string which follows must be length bytes long in JSON format. It is a 2 way protocol.

### Open

Open and return port number to publish to client.

```json
{
  "channel": "open",
  "target": {
    "port": 56802,
    "ip": "192.168.0.141",
    "dtls": {
      "fingerprint": "00:01:ff...",
      "setup": "act"
    },
  },
  "id": "44fd13298e7b427f782ec6cf1ce9482d"
}
```

If "dtls" is populated then fingerprint and setup then will be enforced. The connection will close if the DTLS handshake fails
or the fingerprint does not match to what is expected. "setup" is either "act" or "pass" (default).

id is a transparent id, which is returned with a channel uuid to the server can associate the 2. Commands are then sent using the channel uuid. The port and ip are the expected remote source of the stream.

Response
```json
{
  "id": "44fd13298e7b427f782ec6cf1ce9482d",
  "action": "open",
  "channel": {
    "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
    "port": 10002,
    "ip": "192.168.0.141"
    },
  "status": {
    "channels": {
      "active":2,
      "available":507
    }
  }
}
```

All responses include the server status message.

### Mix

Mix does exactly as it says. It mixes multiple channels to a mixer. It handles 2 at the same time but can be called repeatedly to create scenarios like conferences.

```json
{
  "channel":"mix",
  "uuid":
  [
    "61efa425-7371-41f2-b968-442c54346ccc",
    "4b7d9275-5e4c-42cc-9795-1ae881157544"
  ]
}
```

Currently, we only support an array of 2 uuids. The call should be repeated for more channels to be mixed.

### Close

Close the channel.

```json
{
  "channel": "close",
  "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152"
}
```

ProjectRTP will respond with a confirmation and some stats.

```json
{
  "action":"close",
  "id":"4daf51beeb229c4a69bc124be35a9a0c",
  "uuid":"7dfc35d9-eafe-4d8b-8880-c48f528ec152",
  "stats":
  {
    "in":
    {
      "mos":4.5,
      "count":586,
      "skip":0
    },
    "maxticktimeus":239,
    "meanticktimeus":84
  },
  "status":
  {
    "instance":"a42b1e86-b6c2-4a9b-a839-bc20187663af",
    "channels":
    {
      "active":0,
      "available":509
    }
  }
}
```

#### stats

MOS is only included for our received measurements.

Tick time is the time taken to process RTP data before sending it on its way. We perform a small amount of work when we send and receive
RTP data but the majority of the work is performed on a tick - which gives an indication of load and capability.

Measurements are in uS (hence the us in the naming convention). In the example above the average ticktime was 84uS. A tick occurs on the ptime of a stream (which we only work with a ptime of 20mS).

If there are multiple channels being mixed they will all receive the same tick time as they are mixed in the same tick and this represents the total time for all channels.

### Play

Play a sound 'soup'. This is a list of files to play, potentially including start and stop times of the file to play. ProjectRTP Supports wav files. An example soup:

```json
{
  "channel": "play",
  "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
  "soup": {
    "loop": true,
    "files": [
      { "wav": "filename.wav", "start": 1000, "stop": 4000 }
    ]
  }
}
```

* loop can either be true (continuous - well - INT_MAX) or a number
* loop can be in the main soup (i.e loop through all files) or on a file (i.e. loop through this file 3 times)
* The filename can be either in the param "wav", "pcma", "pcmu", "l168k", "l1616k", "ilbc", "g722". See note below on what they are used for, all contain the name of the file for that format.
* start - number of mS to start playback from (optional)
* stop - number of mS to stop playback (optional)

The start and stop param allows lots of snippets to be included in the same file.

The filename can be included in different param. If you supply the param, the RTP server will assume the file exists and the correct format. The goal with this param is to reduce CPU overhead of any transcoding when playing back a file.

#### Other soup examples

```json
{
  "loop": true,
  "files": [
    { "pcma": "ourpcma.wav", "pcmu": "ourpcmu.wav" }
  ]
}
```

##### 2

Queue announcement

```json
{
  "loop": true,
  "files": [
    { "wav": "ringing.wav", "loop": 6 },
    { "wav": "youare.wav" },
    { "wav": "first.wav" },
    { "wav": "inline.wav" }
  ]
}
```

We may also have combined some wav files:

```json
{
  "loop": true,
  "files": [
    { "wav": "ringing.wav", "loop": 6 },
    { "wav": "youare.wav" },
    { "wav": "ordinals.wav", "start": 3000, "stop": 4500 },
    { "wav": "inline.wav" }
  ]
}
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
finishbelowpower = <int >

used in conjunction with finishbelowpower - used in conjusnction with power thresholds
i.e. power below finishbelowpower before this number of mS has passed
minduration = < int > mSeconds

Must be above minduration and sets a limit on the size of recording
maxduration = < int > mSeconds

Must be 1 or 2
numchannels = < int > count

```json
{
  "channel": "record",
  "uuid":"3f78c0f1-a1e5-4372-87f3-1938f5cb30c4",
  "file":"testfile.wav"
}
```

Record to a filename. UUID is the channel UUID that is returned when you open a file. File is the name - this file will be overwritten if it already exists.

ProjectRTP will respond with

```json
{
  "action":"record",
  "id":"a752dc3c0f5671e067e320d4e632f159",
  "uuid":"ab033164-3a55-4e4d-8f63-dedf71866d29",
  "chaneluuid":"3f78c0f1-a1e5-4372-87f3-1938f5cb30c4",
  "file":"testfile.wav",
  "state":"recording",
  "status":
  {
    "instance":"7ca96e59-6cef-454f-b752-333cdb94112e",
    "channels":
    {
      "active":1,
      "available":508
    }
  }
}
```

In the initial response (which confirms the command) the uuid returned is the uuid of the recording instance. The channeluuid refers back to the channel uuid and the id refers to the channel id you provided to projectrtp when opening the channel. All messages sent to back contain the status so we are updated regarding the workload of the server and also the instance which is the uuid of the projectrtp instance.


```json
{
  "action":"record",
  "uuid":"ab033164-3a55-4e4d-8f63-dedf71866d29",
  "state":"Finished",
  "reason":"finishbelowpower",
  "status":
  {
    "instance":"7ca96e59-6cef-454f-b752-333cdb94112e",
    "channels":
    {
      "active":1,
      "available":508
    }
  }
}
```

This final message is sent when the recording has completed.


## Utils

### Tone generation

In order to make the RTP as scalable as possible, we will not support on the fly tone generation. Currently disk space is much cheaper than CPU resources. 1S of wav data sampled at 8K is 16K. Using soundsoup we can provide wav files for each supported codec and easily loop which requires very little CPU overhead.

What we need to provide is a utility to generate wav files which will generate tones for use in telecoms (i.e. ringing, DTMF etc).

projectrtp --tone 350+440*0.75:1000 dialtone.wav

The format attempts to closely follow the format in https://www.itu.int/dms_pub/itu-t/opb/sp/T-SP-E.180-2010-PDF-E.pdf - although that standard is not the clearest in some respects.

We currently support

* '+' adds 1 or more frequencies together
* '\*' reduces the amplitude (i.e. aptitude multiply by this value, 0.75 is 0.75 x max value)
* ':' values after the colon are cadences, i.e. duration before the tone moves onto the next tone
* '\~' start\~end, for example this can be used in frequency or amplitude, 350+440*0.5\~0:100 would create a tone starting at amp 0.5 reducing to 0 through the course of the 1000mS period

Examples

#### UK Dial tone:
projectrtp --tone 350+440*0.5:1000 dialtone.wav

#### UK Ringing
projectrtp --tone 400+450*0.5/0/400+450*0.5/0:400/200/400/2000 ringing.wav

#### DTMF

||1209Hz|1336Hz|1477Hz|1633Hz|
|---|---|---|---|---|
|697Hz|1|2|3|A|
|770Hz|4|5|6|B|
|852Hz|7|8|9|C|
|941Hz|*|0|#|D|

Example, 1 would mix 1209Hz and 697Hz

projectrtp --tone 697+1209*0.5:400 dtmf1.wav

projectrtp --tone 697+1209*0.5/0/697+1336*0.5/0/697+1477*0.5/0:400/100 dtmf1-3.wav

### --wavinfo

Dump info from wav file header.

### TODO

Format conversion between wav file types (l16, rate pcmu, pcma etc).
