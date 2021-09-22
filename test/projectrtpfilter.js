/*
This test file also has the capability to plot data so we can visualise what
is going on. It is not part of the normal test.

To plot the data to see filters in action:
node ./projectrtpfilter.js plot
*/


const projectrtp = require( "../src/build/Release/projectrtp" )
const expect = require( "chai" ).expect

function int16bebuffer2array( inbuffer ) {
  let r = []
  for( let i = 0; i < inbuffer.length; i = i + 2 ) {
    r.push( inbuffer.readInt16BE( i ) )
  }
  return r
}

function gentone( durationseconds = 0.25, tonehz = 100, samplerate = 16000, amp = 15000 ) {
  const tonebuffer = Buffer.alloc( samplerate*durationseconds, 0 )

  for( let i = 0; i < tonebuffer.length / 2; i++ ) {
    let val = Math.sin( ( i / samplerate ) * Math.PI * tonehz ) * amp
    tonebuffer.writeInt16BE( val, i * 2 )
  }

  return tonebuffer
}

/* tonebuffer should be same sampling rate */
function addtone( tonebuffer, tonehz = 100, samplerate = 16000, amp = 15000 ) {
  for( let i = 0; i < tonebuffer.length / 2; i++ ) {
    let val = Math.sin( ( i / samplerate ) * Math.PI * tonehz ) * amp
    let currentval = tonebuffer.readInt16BE( i * 2 )
    tonebuffer.writeInt16BE( val + currentval, i * 2 )
  }

  return tonebuffer
}

let args = process.argv.slice( 2 )
if( args.length > 0 && "plot" == args[ 0 ] ) {
  const plot = require( "nodeplotlib" )

  let p = gentone( 0.25, 15000 )
  addtone( p )

  let layout1 = {
    "title": "Input data - 100Hz and 10Khz mixed",
    "xaxis": {
      "title": "Sample",
    }
  }

  let data1 = [ {
    y: int16bebuffer2array( p ),
    type: "scatter"
  } ]

  plot.stack( data1, layout1 )

  projectrtp.rtpfilter.filterlowfir( p )

  let layout2 = {
    "title": "Output of FIR Filter - 12Khz removed",
    "xaxis": {
      "title": "Sample",
    }
  }

  let data2 = [ {
    y: int16bebuffer2array( p ),
    type: "scatter"
  } ]

  plot.stack( data2, layout2 )
  plot.plot()

} else {
  describe( "rtpfilter", function() {

    it( `Test low pass filter - remove 12K signal`, async function() {
      let intone = gentone( 0.25, 12000 )
      expect( projectrtp.rtpfilter.filterlowfir( intone ) ).to.be.true

      /* don't start right at the beggining as there is some impulse response */
      for( let i = 50; i < intone.length; i = i + 2 ) {
        expect( intone.readInt16BE( i ) ).to.be.within( -20, 20 )
      }
    } )
  } )
}
