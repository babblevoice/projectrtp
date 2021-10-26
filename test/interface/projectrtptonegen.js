/*
TODO
Call the function and check we have output files. Further testing.
*/

const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp

describe( "tonegen", function() {
  it( `tone.generate exists`, async function() {
    expect( projectrtp.tone.generate ).to.be.an( "function" )
  } )
} )


/**
@summary functions for tone manipulation.
@memberof projectrtp
@hideconstructor
*/

class tone {

  /**
  @summary Generate tone from definition string
  @param {string} tone - tone description
  @param {string} file - wav file to append the data to, it will create new if it doesn't exist.
  @returns {boolean}
  @description
We need to be able to generate tones. This is following the [standard](https://www.itu.int/dms_pub/itu-t/opb/sp/T-SP-E.180-2010-PDF-E.pdf).
Looping will be handled by soundsoup. Our generated file only needs to handle one cycle of the tone.

Our goal is to be efficient, so we do not generate this on the fly - most tones will be generated into wav files and
played when required.

If we want to play a tone continuously we should find a nicely looped file (e.g 1S will mean all frequencies in the
file will hit zero at the end of the file). This would simplify our generation.

In the standard we have definitions such as:

### United Kingdom of Great Britain and Northern Ireland
 - Busy tone - 400 0.375 on 0.375 off
 - Congestion tone - 400 0.4 on 0.35 off 0.225 on 0.525 off
 - Dial tone - 50//350+440 continuous
 - Number unobtainable tone - 400 continuous
 - Pay tone - 400 0.125 on 0.125 off
 - Payphone recognition tone - 1200/800 0.2 on 0.2 off 0.2 on 2.0 off
 - Ringing tone - 400+450//400x25//400x16 2/3 0.4 on 0.2 off 0.4 on 2.0 off

i.e. Tone - Frequency - Cadence

Frequency in Hz
 - f1×f2 f1 is modulated by f2
 - f1+f2 the juxtaposition of two frequencies f1 and f2 without modulation
 - f1/f2 f1 is followed by f2
 - f1//f2 in some exchanges frequency f1 is used and in others frequency f2 is used.
 - Cadence in seconds: ON – OFF

Try to keep our definitions as close to the standard. We also have to introduce some other items:

 - Amplitude
 - Change (in frequency or amplitude) - frequency can be handled by modulated

Take ringing tone:

400+450//400x25//400x16 2/3 0.4 *on 0.2 off 0.4 on 2.0 off*

We can ignore the // in our definition as we can simply choose the most common one.
So either 400+450 or 400x25
*Three does not appear ot be anything in the standard relating to the 2/3?*

Amplitude can be introduced by *
so

400+450 becomes 400+450*0.75 (every frequency will have its amplitude reduced).
400x25*0.75 is then also suported.

Increasing tones such as:
950/1400/1800

Cadence
950/1400/1800/0:333/333/333/1000
Note, we have introduced a final /0 to indicate silence. The cadences will iterated through for every / in the frequency list and is in mS (the standard lists in seconds). We don't need to support loops as soundsoup supports loops.
For:
950/1400/1800/0:333
Means each section will be 333mS.

Change
400+450*0.75~0 will reduce the amplitude from 0.75 to 0 during that cadence period
400~450 will increase the frequency during that cadence period

Note 400+450x300 is not supported.

UK Examples:

 - 350+440*0.5:1000 Dial tone
 - 400+450*0.5/0/400+450*0.5/0:400/200/400/2000 Ringing
 - 697+1209*0.5/0/697+1336*0.5/0/697+1477*0.5/0/697+1633*0.5/0:400/100 DTMF 123A
 - 770+1209*0.5/0/770+1336*0.5/0/770+1477*0.5/0/770+1633*0.5/0:400/100 DTMF 456B
 - 852+1209*0.5/0/852+1336*0.5/0/852+1477*0.5/0/852+1633*0.5/0:400/100 DTMF 789C
 - 941+1209*0.5/0/941+1336*0.5/0/941+1477*0.5/0/941+1633*0.5/0:400/100 DTMF *0#D
 - 440:1000 Unobtainable
 - 440/0:375/375 Busy
 - 440/0:400/350/225/525 Congestion
 - 440/0:125/125 Pay

```
tone.generate( "350+440*0.5:1000", "uksounds.wav" )
tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "uksounds.wav" )
...
```
A sound soup can then be used to index the times within the wav file.
  */
  generate(){}
}
