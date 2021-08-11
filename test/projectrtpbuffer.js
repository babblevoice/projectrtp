
const should = require( "chai" ).should()
const expect = require( "chai" ).expect

const projectrtp = require( "../src/build/Release/projectrtp" )

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
  describe( "push/pop", function() {
    it( `push data and check pops and peek happen at the right time`, async function() {

      let b = projectrtp.rtpbuffer.create()

      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x03, 0x02, 0x00, 0x00, 0x00, 0x00 ] ) } ) // out of range
      expect( b.pop() ).to.be.undefined
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x03, 0xff, 0xff, 0x00, 0x00, 0x00 ] ) } ) // out of range

      for ( let step = 0; step < 4; step++ ) {
        let pk = b.peek()
        /* the sequence number should not change */
        expect( pk[ 2 ] ).to.equal( 1 )
        expect( pk[ 3 ] ).to.equal( 0 )
      }

      for ( let step = 0; step < 4; step++ ) {
        let pk = b.pop()
        expect( pk[ 2 ] ).to.equal( 1 )
        expect( pk[ 3 ] ).to.equal( step )
      }

      for ( let step = 0; step < 15; step++ ) {
        expect( b.pop() ).to.be.undefined
      }
    } )

    it( `fill then empty to see we can restart at a different sn`, async function() {

      let b = projectrtp.rtpbuffer.create()

      for ( let step = 0; step < 10; step++ ) {
        b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x01, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
      }

      for ( let step = 0; step < 5; step++ ) {
        expect( b.pop() ).to.be.undefined
      }

      for ( let step = 0; step < 10; step++ ) {
        let pk = b.pop()
        expect( pk[ 2 ] ).to.equal( 1 )
        expect( pk[ 3 ] ).to.equal( step )
      }

      /* now restart from a much higher sn */
      for ( let step = 0; step < 10; step++ ) {
        b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x02, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
      }

      for ( let step = 0; step < 5; step++ ) {
        expect( b.pop() ).to.be.undefined
      }

      for ( let step = 0; step < 10; step++ ) {
        let pk = b.pop()
        expect( pk[ 2 ] ).to.equal( 2 )
        expect( pk[ 3 ] ).to.equal( step )
      }
    } )

    it( `try inserting out of range sn and ensure they are ignored`, async function() {
      let b = projectrtp.rtpbuffer.create()

      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 100, 0x00, 0x00, 0x00, 0x00 ] ) } )
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 116, 0x00, 0x00, 0x00, 0x00 ] ) } )
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 84, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
      expect( b.pop() ).to.be.undefined
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 105, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
      b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, 104, 0x00, 0x00, 0x00, 0x00 ] ) } )
      expect( b.pop() ).to.be.undefined
      expect( b.pop() ).to.be.undefined


      let pk = b.pop()
      expect( pk[ 2 ] ).to.equal( 0 )
      expect( pk[ 3 ] ).to.equal( 100 )

      for ( let step = 0; step < 3; step++ ) {
        expect( b.pop() ).to.be.undefined
      }

      pk = b.pop()
      expect( pk[ 2 ] ).to.equal( 0 )
      expect( pk[ 3 ] ).to.equal( 104 )

      pk = b.pop()
      expect( pk[ 2 ] ).to.equal( 0 )
      expect( pk[ 3 ] ).to.equal( 105 )

    } )

    it( `pump some data through with wrap of sn`, async function() {
      let b = projectrtp.rtpbuffer.create()

      for ( let step = 0; step < 5; step++ ) {
        b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0xff, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
        expect( b.pop() ).to.be.undefined
      }

      for ( let step = 5; step < 256; step++ ) {
        b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0xff, step, 0x00, 0x00, 0x00, 0x00 ] ) } )
        let pk = b.pop()

        expect( pk[ 2 ] ).to.equal( 255 )
        expect( pk[ 3 ] ).to.equal( step - 5 )
      }

      /* wrap */
      for ( let step = 0; step < 100; step++ ) {
        b.push( { "payload": Buffer.from( [ 0x80, 0x00, 0x00, step, 0x00, 0x00, 0x00, 0x00 ] ) } )

        let pk = b.pop()
        if( step < 5 ) {
          /* remaining data from previous loop */
          expect( pk[ 2 ] ).to.equal( 255 )
          expect( pk[ 3 ] ).to.equal( 256 - ( 5 - step ) )
        } else {
          expect( pk[ 2 ] ).to.equal( 0 )
          expect( pk[ 3 ] ).to.equal( step - 5 )
        }
      }

    } )
  } )

  before( () => {
    projectrtp.run()
  } )

  after( async () => {
    await projectrtp.shutdown()
  } )
} )