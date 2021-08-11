
const should = require( "chai" ).should()
const expect = require( "chai" ).expect

const projectrtp = require( "../src/build/Debug/projectrtp" )

const dgram = require( "dgram" )

describe( "rtpchannel", function() {

  it( `structure of rtpchannel is correct`, async function() {

    expect( projectrtp.rtpchannel ).to.be.an( "object" )
    expect( projectrtp.rtpchannel.create ).to.be.an( "function" )

  } )

  it( `call create channel and check the structure of the returned object`, async function() {

    this.timeout( 2000 )
    this.slow( 1500 )

    let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": 20000 } } )
    expect( channel ).to.be.an( "object" )

    expect( channel.close ).to.be.an( "function" )
    expect( channel ).to.have.property( "port" ).that.is.a( "number" )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1000 ) } )
    channel.close()
  } )

  it( `call create channel echo`, function( done ) {

    /* create our RTP/UDP endpoint */
    const ourport = 20000
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )
    server.bind( ourport )

    this.timeout( 3000 )
    this.slow( 2500 )

    let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": ourport } }, function( d ) {

      if( "close" === d.action ) {
        console.log( d )
        expect( receviedpkcount ).to.equal( 50 )
        expect( d.stats.in.count ).to.equal( 50 )
        expect( d.stats.out.count ).to.equal( 50 )

        server.close()
        done()
      }
    } )
    expect( channel ).to.be.an( "object" )

    expect( channel.close ).to.be.an( "function" )
    expect( channel ).to.have.property( "port" ).that.is.a( "number" )

    expect( channel.echo() ).to.be.true

    function sendpk( sn ) {
      setTimeout( () => {
        let payload = Buffer.alloc( 172 - 8 ).fill( 0 )
        let ts = sn * 160
        let tsparts = []
        /* portability? */
        tsparts[ 3 ] = ( ts & 0xff000000 ) >> 24
        tsparts[ 2 ] = ( ts & 0xff0000 ) >> 16
        tsparts[ 1 ] = ( ts & 0xff00 ) >> 8
        tsparts[ 0 ] = ( ts & 0xff )

        let rtppacket = Buffer.concat( [
          Buffer.from( [ 0x80, 0x00, 0x00, 100 + sn, tsparts[ 3 ], tsparts[ 2 ], tsparts[ 1 ], tsparts[ 0 ] ] ),
          payload ] )

        server.send( rtppacket, channel.port, "localhost" )
      }, sn * 20 )
    }
    /* send a packet every 20mS x 50 */
    for( i = 0;  i < 50; i ++ ) {
      sendpk( i )
    }

    setTimeout( () => channel.close(), 2000 )

  } )

  it( `call create channel echo and skip some packets`, function( done ) {
    // TODO
  } )

  it( `call create channel echo and send out of order packets`, function( done ) {
    // TODO
  } )

  it( `call create channel echo and send packets outside of window`, function( done ) {
    // TODO
  } )

  it( `call create channel echo and simulate a stalled connection`, function( done ) {
    // TODO
  } )

  it( `call create channel echo whilst wrapping the sn `, function( done ) {
    // TODO
  } )

  before( () => {
    console.log("calling run")
    projectrtp.run()
  } )

  after( async () => {
    console.log("calling shutdown")
    await projectrtp.shutdown()
  } )
} )
