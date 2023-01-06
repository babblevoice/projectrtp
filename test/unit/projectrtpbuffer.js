
const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp


/* From RFC 1889
  0                   1                   2                   3
  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |V=2|P|X|  CC   |M|     PT      |       sequence number         |
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |                           timestamp                           |
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
  |           synchronization source (SSRC) identifier            |
  +=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
  |            contributing source (CSRC) identifiers             |
  |                             ....                              |
  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
*/

describe( "rtpbuffer", function() {
  it( "Check functions exist", async function() {
    expect( projectrtp.rtpbuffer.create ).to.be.an( "function" )
  } )

  it( "push data and check pops and peek happen at the right time", async function() {

    const b = projectrtp.rtpbuffer.create()

    for( let v = 0; 10 > v; v++ ) {
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x01, v, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
    }

    /* These should be dropped */
    b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x03, 0x02, 0x00, 0x00, 0x00, 0x00 ] ) } ) // out of range
    b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x03, 0xff, 0xff, 0x00, 0x00, 0x00 ] ) } ) // out of range

    for ( let step = 0; 4 > step; step++ ) {
      const pk = b.peek()
      /* the sequence number should not change */
      expect( pk[ 2 ] ).to.equal( 1 )
      expect( pk[ 3 ] ).to.equal( 0 )
    }

    for ( let step = 0; 10 > step; step++ ) {
      const pk = b.pop()
      expect( pk[ 2 ] ).to.equal( 1 )
      expect( pk[ 3 ] ).to.equal( step )
    }

    for ( let step = 0; 15 > step; step++ ) {
      expect( b.pop() ).to.be.undefined
    }
  } )

  it( "fill then empty to see we can restart at a different sn", async function() {

    const b = projectrtp.rtpbuffer.create()

    for ( let step = 0; 5 > step; step++ ) {
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x01, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
    }

    for ( let step = 0; 10 > step; step++ ) {
      expect( b.pop() ).to.be.undefined
    }

    for ( let step = 5; 15 > step; step++ ) {
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x01, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
    }

    for ( let step = 0; 15 > step; step++ ) {
      const pk = b.pop()
      expect( pk[ 2 ] ).to.equal( 1 )
      expect( pk[ 3 ] ).to.equal( step )
    }

    /* now restart from a much higher sn */
    for ( let step = 0; 10 > step; step++ ) {
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x02, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
    }

    for ( let step = 0; 10 > step; step++ ) {
      const pk = b.pop()
      expect( pk[ 2 ] ).to.equal( 2 )
      expect( pk[ 3 ] ).to.equal( step )
    }
  } )

  it( "try inserting out of range sn and ensure they are ignored", async function() {
    const b = projectrtp.rtpbuffer.create()

    b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 100, 0x00, 0x00, 0x00, 0x00 ] ) } )
    b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 116, 0x00, 0x00, 0x00, 0x00 ] ) } )
    b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 84, 0x00, 0x00, 0x00, 0x00 ] ) } )
    expect( b.pop() ).to.be.undefined
    expect( b.pop() ).to.be.undefined
    b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 105, 0x00, 0x00, 0x00, 0x00 ] ) } )
    expect( b.pop() ).to.be.undefined
    b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 104, 0x00, 0x00, 0x00, 0x00 ] ) } )

    for ( let step = 0; 7 > step; step++ ) {
      expect( b.pop() ).to.be.undefined
    }


    let pk = b.pop()
    expect( pk[ 2 ] ).to.equal( 0 )
    expect( pk[ 3 ] ).to.equal( 100 )

    for ( let step = 0; 3 > step; step++ ) {
      expect( b.pop() ).to.be.undefined
    }

    pk = b.pop()
    expect( pk[ 2 ] ).to.equal( 0 )
    expect( pk[ 3 ] ).to.equal( 104 )

    pk = b.pop()
    expect( pk[ 2 ] ).to.equal( 0 )
    expect( pk[ 3 ] ).to.equal( 105 )

  } )

  it( "pump some data through with wrap of sn", async function() {
    const b = projectrtp.rtpbuffer.create()

    for ( let step = 0; 10 > step; step++ ) {
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0xff, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
    }

    for ( let step = 10; 256 > step; step++ ) {
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0xff, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
      const pk = b.pop()

      expect( pk[ 2 ] ).to.equal( 255 )
      expect( pk[ 3 ] ).to.equal( step - 10 )
    }

    /* wrap */
    for ( let step = 0; 100 > step; step++ ) {
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, step, 0x00, 0x00, 0x00, 0x00 ] ) } )

      const pk = b.pop()
      if( 10 > step ) {
        /* remaining data from previous loop */
        expect( pk[ 2 ] ).to.equal( 255 )
        expect( pk[ 3 ] ).to.equal( 256 - ( 10 - step ) )
      } else {
        expect( pk[ 2 ] ).to.equal( 0 )
        expect( pk[ 3 ] ).to.equal( step - 10 )
      }
    }
  } )

  it( "ensure our peeked buffer is maintained", async function() {
    const b = projectrtp.rtpbuffer.create()
    const buffersize = b.size()
    const halffull = Math.floor( buffersize / 2 )

    let step = 0
    for ( ; step < halffull; step++ ) {
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0xff, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
      b.pop()
    }

    let pk = b.peek()
    expect( pk[ 3 ] ).to.equal( 0 )

    /* trash the whole buffer */
    for ( ; step < buffersize; step++ ) {
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0xff, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
    }

    /* our peeked should be safe */
    b.poppeeked()
    expect( pk[ 3 ] ).to.equal( 0 )

    pk = b.peek()
    b.poppeeked()
    expect( pk[ 3 ] ).to.equal( 1 )

    pk = b.peek()
    b.poppeeked()
    expect( pk[ 3 ] ).to.equal( 2 )

  } )
} )
