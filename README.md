# ProjectRTP

A simple RTP server which offers functionality to process RTP data streams and mix them. Channels will be able to be mixed with other channels or local functions like recording or file playback. Instructions to create and manipulate RTP streams via a TCP connection (control connection). Enabling the ability to have 1 SIP/control server to manage multiple RTP servers to increase scalability of the system.

This is currently work in progress.

## Dependencies

* ilbc-devel
* spandsp-devel
* boost

# Build

cd src
make

This will build the executable and also generate some UK sounds.

## Codecs

ProjectRTP supports transcoding for the following CODECs

* PCMA
* PCMU
* G722
* iLBC

## Control

The server is managed via a control socket. I am designing it so that multiple RTP servers can connect to a call control server to create a flexible voice switch.

projectrtp --fg --pa 192.168.0.141 --connect 127.0.0.1

See projectrtpmain.cpp for all options. In this example, the --pa is the IP address to instruct the control server which IP to include in SDP. --connect is the address of the control server. Node library coming soon.

### Protocol

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
    "ip": "192.168.0.141"
    },
  "id": "44fd13298e7b427f782ec6cf1ce9482d"
}
```

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

All responses include the server status message. I intent to add further information regarding load to enable the control server more information to improve decision making on which Project RTP instance to use.

### Close

Close the channel.

```json
{
  "channel": "close",
  "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152"
}
```

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

```json
{
  "channel": "record",
  "uuid":"3f78c0f1-a1e5-4372-87f3-1938f5cb30c4",
  "file":"testfile.wav"
}

Record to a filename. UUID is the channel UUID that is returned when you open a file. File is the name - this file will be overwritten if it already exists. 
```

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
