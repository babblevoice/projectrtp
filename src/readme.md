# RTP Server

The RTP server offers functionality to process RTP data streams and mix them. Channels will be able to be mixed with other channels or local functions like recording or file playback.

## Dependencies

* ilbc-devel
* spandsp-devel

## API

### Channels
*POST /channel*

Channel functions which can create and destroy channels. Channels are pre-allocated at start-up and when a channel is requested by our control server is pulled from the pool and put to use.

This call returns a JSON object:

```json
{
  "channel": "<uuid>",
  "port": 10000,
  "ip": "8.8.8.8"
}
```

*DELETE /channel/<uuid>*

As it says - returns 200 ok on completion.

*PUT /channel/target/<uuid>*

This sets the remote address - where we transmit RTP UDP data to. It is not always needed - as if we receive UDP data from our client we will use that address in preference anyway.

```json
{
  "port": 12000,
  "ip": "8.8.4.4"
}
```

### Mix

*PUT /channel/<uuid>/mix/<uuid>*

Configures both channels to mix together.

### Play

*PUT /channel/<uuid>/play*

We have designed a format to try to be as flexible enough for different situations. As part of the principle is to fire and forget - i.e. send an instruction to the RTP server and let it run until something changes - at which point the RTP server will be updated.

It currently only supports playing to an unmixed channel. Plays a soup of files to the connected channel. A soup is a recipe of files to play to the user:

```json
{
  "loop": true,
  "files": [
    { "wav": "filename.wav", "start": 1000, "stop": 4000 }
  ]
}
```

* loop can either be true (continuous - well - INT_MAX) or a number
* loop can be in the main soup (i.e loop through all files) or on a file (i.e. loop through this file 3 times)
* The filename can be either in the param "wav", "pcma", "pcmu", "l168k", "l1616k", "ilbc", "g722". See note below on what they are used for, all contain the name of the file for that format.
* start - number of mS to start playback from (optional)
* stop - number of mS to stop playback (optional)

The start and stop param allows lots of snippets to be included in the same file.

The filename can be included in different param. If you supply the param, the RTP server will assume the file exists and the correct format. The goal with this param is to reduce CPU overhead of any transcoding when playing back a file. 

#### Examples

##### 1

If you only allow connections which use pcma or pcmu then you have a few options

1. Supply a wav file in l168k format. This can be supplied specifically in the l168k param or wav. Overhead for transcoding l168k to pcma or pcmu is not high.
2. Supply 2 wav files, 1 containing pcma data and the other pcmu. Then provide "pcmu": "ourpcmu.wav" and "pcma": "ourpcma.wav"

Notes, it would not be prudent to supply a l1616k formated file. As part of the transcoding the data has to be down-sampled which includes a low pass filter to remove high frequencies first - this has a higher CPU overhead.

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

### Transcoding

For some scenarios we may not need to transcode. However we may need to transcode to more than one or more other CODECs. For example, we receive PCMA, which is part of a conference which there are 2 other clients. One asking for G722 and the other PCMU. This may also be the case in the future if we handle video.

As this is the case, the sender channel should always be responsible for transcoding. This way as the sender will know about it's multiple receivers it can transcode for all receivers. If there are 2 requiring the same CODEC we only need to transcode once. We can also keep the intermediate L16 for the different CODECs.

### Measurement

How are we going to measure it. It - the workload. Using any strategy it would be impossible to balance workload across all CPUs evenly. In fact - a multi thread application would probably be more successful at this than this strategy. We probably need to report back to our control server a number indicative of if we are at capacity or not.

## Control Server

Each control server can farm out RTP sessions to any number of RTP servers it knows about. It should have control about which server it uses and also have the ability to spin up extra resources if required in cloud environments.

Some of the challenges which is bothering me at the moment:

* If we decide to shut down some of our RTP resource but one server has a call still hanging on what do we do?
  * We could re-invite that call to use a different server but that would cause problems if a resource was being used for that call, for example, the call is being recorded to a local disk before upload. This may mean we have to limit this type of function to mountable shared disk resources.
* Each RTP server should report back how stressed it is. This information can be used in the decision making of where to place a call to.

Each channel needs a control URL so it can send information back to the control server if required. For example, DTMF, or file finished playing and so on.

## Utils

### Tone generation

In order to make the RTP as scalable as possible, we will not support on the fly tone generation. Currently disk space is much cheaper than CPU resources. 1S of wav data sampled at 8K is 16K. Using soundsoup we can provide wav files for each supported codec and easily loop which requires very little CPU overhead.

What we need to provide is a utility to generate wav files which will generate tones for use in telecoms (i.e. ringing, DTMF etc).

project-rtp --tone 350+440*0.75:1000 dialtone.wav

The format attempts to closely follow the format in https://www.itu.int/dms_pub/itu-t/opb/sp/T-SP-E.180-2010-PDF-E.pdf - although that standard is not the clearest in some respects.

We currently support

* '+' adds 1 or more frequencies together
* '*' reduces the amplitude (i.e. aptitude multiply by this value, 0.75 is 0.75 x max value)
* ':' values after the colon are cadences, i.e. duration before the tone moves onto the next tone
* '\~' start\~end, for example this can be used in frequency or amplitude, 350+440*0.5\~0:100 would create a tone starting at amp 0.5 reducing to 0 through the course of the 1000mS period

Examples

#### UK Dial tone:
project-rtp --tone 350+440*0.5:1000 dialtone.wav

#### UK Ringing
project-rtp --tone 400+450*0.5/0/400+450*0.5/0:400/200/400/2000 ringing.wav

#### DTMF

||1209Hz|1336Hz|1477Hz|1633Hz|
|---|---|---|---|---|
|697Hz|1|2|3|A|
|770Hz|4|5|6|B|
|852Hz|7|8|9|C|
|941Hz|*|0|#|D|

Example, 1 would mix 1209Hz and 697Hz

project-rtp --tone 697+1209*0.5:400 dtmf1.wav

project-rtp --tone 697+1209*0.5/0/697+1336*0.5/0/697+1477*0.5/0:400/100 dtmf1-3.wav
