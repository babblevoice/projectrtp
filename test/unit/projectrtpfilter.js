/*
This test file also has the capability to plot data so we can visualise what
is going on. It is not part of the normal test.

To plot the data to see filters in action:
node ./projectrtpfilter.js plot
*/
const projectrtp = require( "../../index.js" ).projectrtp
const expect = require( "chai" ).expect

function gentone( durationseconds = 0.25, tonehz = 100, samplerate = 16000, amp = 15000 ) {
  const tonebuffer = Buffer.alloc( samplerate*durationseconds, 0 )

  for( let i = 0; i < tonebuffer.length / 2; i++ ) {
    const val = Math.sin( ( i / samplerate ) * Math.PI * tonehz ) * amp
    tonebuffer.writeInt16BE( val, i * 2 )
  }

  return tonebuffer
}

describe( "rtpfilter", function() {

  it( "Test low pass filter - remove 12K signal", async function() {
    const intone = gentone( 0.25, 12000 )
    expect( projectrtp.rtpfilter.filterlowfir( intone ) ).to.be.true

    /* don't start right at the beggining as there is some impulse response */
    for( let i = 50; i < intone.length; i = i + 2 ) {
      expect( intone.readInt16BE( i ) ).to.be.within( -20, 20 )
    }
  } )
} )
