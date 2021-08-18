
const should = require( "chai" ).should()
const expect = require( "chai" ).expect

const projectrtp = require( "../src/build/Release/projectrtp" )

const dgram = require( "dgram" )

/* helper functions */
function sendpk( sn, sendtime, dstport, server, ssrc = 25 ) {

  setTimeout( () => {
    let payload = Buffer.alloc( 172 - 12 ).fill( sn & 0xff )
    let ts = sn * 160
    let tsparts = []
    /* portability? */
    tsparts[ 3 ] = ( ts & 0xff000000 ) >> 24
    tsparts[ 2 ] = ( ts & 0xff0000 ) >> 16
    tsparts[ 1 ] = ( ts & 0xff00 ) >> 8
    tsparts[ 0 ] = ( ts & 0xff )

    let snparts = []
    sn = ( sn + 100 ) % ( 2**16 ) /* just some offset */
    snparts[ 0 ] = sn & 0xff
    snparts[ 1 ] = sn >> 8

    let ssrcparts = []
    ssrcparts[ 3 ] = ( ssrc & 0xff000000 ) >> 24
    ssrcparts[ 2 ] = ( ssrc & 0xff0000 ) >> 16
    ssrcparts[ 1 ] = ( ssrc & 0xff00 ) >> 8
    ssrcparts[ 0 ] = ( ssrc & 0xff )


    let rtppacket = Buffer.concat( [
      Buffer.from( [
        0x80, 0x00,
        snparts[ 1 ], snparts[ 0 ],
        tsparts[ 3 ], tsparts[ 2 ], tsparts[ 1 ], tsparts[ 0 ],
        ssrcparts[ 3 ], ssrcparts[ 2 ], ssrcparts[ 1 ], ssrcparts[ 0 ]
       ] ),
      payload ] )

    server.send( rtppacket, dstport, "localhost" )
  }, sendtime * 20 )
}

/* Tests */
describe( "rtpchannel", function() {

  it( `check channel stats at the start`, function( done ) {
    let s = projectrtp.rtpchannel.stats()

    expect( s.available ).to.equal( 5000 )
    expect( s.current ).to.equal( 0 )
    done()
  } )

  it( `structure of rtpchannel is correct`, async function() {

    expect( projectrtp.rtpchannel ).to.be.an( "object" )
    expect( projectrtp.rtpchannel.create ).to.be.an( "function" )

  } )

  it( `call create channel and check the structure of the returned object`, async function() {

    this.timeout( 2000 )
    this.slow( 1500 )

    let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": 20000, "codec": 0 } } )
    expect( channel ).to.be.an( "object" )

    expect( channel.close ).to.be.an( "function" )
    expect( channel ).to.have.property( "port" ).that.is.a( "number" )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1000 ) } )
    channel.close()
  } )

  it( `call create channel echo`, function( done ) {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", function() {

      let ourport = server.address().port

      let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {

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

      /* send a packet every 20mS x 50 */
      for( let i = 0;  i < 50; i ++ ) {
        sendpk( i, i, channel.port, server )
      }

      setTimeout( () => channel.close(), 2000 )
    } )
  } )

  it( `create channel echo and skip some packets`, function( done ) {

    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", function() {

      let ourport = server.address().port

      let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {
          expect( receviedpkcount ).to.equal( 50 - 6 )
          expect( d.stats.in.count ).to.equal( 50 - 6 )
          expect( d.stats.out.count ).to.equal( 50 - 6 )

          server.close()
          done()
        }
      } )
      expect( channel ).to.be.an( "object" )

      expect( channel.close ).to.be.an( "function" )
      expect( channel ).to.have.property( "port" ).that.is.a( "number" )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0;  i < 50; i ++ ) {
        if( i in { 3:0, 13:0, 23:0, 24:0, 30:0, 49:0 } ) continue
        sendpk( i, i, channel.port, server )
      }

      setTimeout( () => channel.close(), 2000 )
    } )
  } )

  it( `create channel echo and send out of order packets`, function( done ) {
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0

    var lastsn = -1
    var lastts = -1
    var totalsndiff = 0
    var totaltsdiff = 0
    server.on( "message", function( msg, rinfo ) {
      let sn = 0
      sn = msg[ 2 ] << 8
      sn = sn | msg[ 3 ]

      let ts = 0
      ts = msg[ 4 ] << 24
      ts = ts | ( msg[ 5 ] << 16 )
      ts = ts | ( msg[ 6 ] << 8 )
      ts = ts | msg[ 7 ]

      if( -1 !== lastsn ) {
        totalsndiff += sn - lastsn - 1
        totaltsdiff += ts - lastts - 160
      }

      lastsn = sn
      lastts = ts

      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", function() {

      let ourport = server.address().port

      let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {

          expect( receviedpkcount ).to.equal( 50 )
          expect( d.stats.in.count ).to.equal( 50 )
          expect( d.stats.out.count ).to.equal( 50 )
          expect( totalsndiff ).to.equal( 0 ) // received should be reordered
          expect( totaltsdiff ).to.equal( 0 )

          server.close()
          done()
        }
      } )
      expect( channel ).to.be.an( "object" )
      expect( channel.echo() ).to.be.true
      expect( channel.close ).to.be.an( "function" )
      expect( channel ).to.have.property( "port" ).that.is.a( "number" )

      /* send a packet every 20mS x 50 */
      let sns = [ 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
        11, 13, 14, 12, 15, 16, 17, 18, 19, 20,
        21, 22, 23, 25, 24, 26, 27, 28, 29, 30,
        31, 37, 33, 34, 36, 35, 32, 38, 39, 40,
        41, 42, 43, 44, 45, 46, 47, 48, 49 ]

      sns.forEach( function( e, i ) {
          sendpk( e, i, channel.port, server )
      } )

      setTimeout( () => channel.close(), 2000 )
    } )
  } )

  it( `create channel echo and send packets outside of window`, function( done ) {
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", function() {

      let ourport = server.address().port

      let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {

          expect( receviedpkcount ).to.equal( 47 )
          expect( d.stats.in.count ).to.equal( 50 )
          expect( d.stats.in.dropped ).to.equal( 3 )
          expect( d.stats.out.count ).to.equal( 47 )

          server.close()
          done()
        }
      } )
      expect( channel ).to.be.an( "object" )

      expect( channel.close ).to.be.an( "function" )
      expect( channel ).to.have.property( "port" ).that.is.a( "number" )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      let sns = [ 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
        11, 13, 14, 12, 15, 16, 17, 18, 19, 20,
        21, 22, 23, 25, 24, 26, 27, 28, 29, 100,
        31, 37, 33, 34, 36, 35, 32, 38, 39, 400,
        41, 42, 43, 44, 45, 46, 47, 48, 2 ]

      sns.forEach( function( e, i ) {
          sendpk( e, i, channel.port, server )
      } )

      setTimeout( () => channel.close(), 2000 )
    } )
  } )

  it( `create channel echo and simulate a stalled connection`, function( done ) {
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0

    var firstsn = 0
    var lastsn = -1
    var lastts = -1
    var totalsndiff = 0
    var totaltsdiff = 0
    server.on( "message", function( msg, rinfo ) {
      let sn = 0
      sn = msg[ 2 ] << 8
      sn = sn | msg[ 3 ]

      let ts = 0
      ts = msg[ 4 ] << 24
      ts = ts | ( msg[ 5 ] << 16 )
      ts = ts | ( msg[ 6 ] << 8 )
      ts = ts | msg[ 7 ]

      if( -1 !== lastsn ) {
        totalsndiff += sn - lastsn - 1
        totaltsdiff += ts - lastts - 160
      } else {
        firstsn = sn
      }

      lastsn = sn
      lastts = ts

      receviedpkcount++
    } )

    this.timeout( 10000 )
    this.slow( 8000 )

    server.bind()
    server.on( "listening", function() {

      let ourport = server.address().port

      let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {

          /*
            We should receive
            50 from the first batch
            10 from the catchup as most will be dropped
            130 from the final batch as it will take 20 to empty the buffer then resume
          */
          expect( receviedpkcount ).to.equal( d.stats.out.count )
          expect( d.stats.in.count ).to.equal( 300 )
          expect( d.stats.out.count ).to.equal( 190 )
          expect( totalsndiff ).to.equal( 0 ) // received should be reordered
          expect( totaltsdiff ).to.equal( 17600 )
          expect( lastsn - firstsn ).to.equal( 189 )

          server.close()
          done()
        }
      } )
      expect( channel ).to.be.an( "object" )

      expect( channel.close ).to.be.an( "function" )
      expect( channel ).to.have.property( "port" ).that.is.a( "number" )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( var i = 0; i < 50; i++ ) {
        sendpk( i, i, channel.port, server )
      }

      /* pause then catchup */
      for( ; i < 150; i++ ) {
        sendpk( i, 150, channel.port, server )
      }

      /* resume */
      for( ; i < 300; i++ ) {
        sendpk( i, i, channel.port, server )
      }

      setTimeout( () => channel.close(), 7000 )
    } )
  } )

  it( `create channel echo whilst wrapping the sn `, function( done ) {
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", function() {

      let ourport = server.address().port

      let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {

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

      /* send a packet every 20mS x 50 */
      for( let i = 0 ;  i < 50; i ++ ) {
        let sn = i + ( 2**16 ) - 25
        sendpk( sn, i, channel.port, server )
      }

      setTimeout( () => channel.close(), 2000 )
    } )
  } )


  it( `create channel echo and incorrectly change the ssrc`, function( done ) {
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", function() {

      let ourport = server.address().port

      let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {

          expect( receviedpkcount ).to.equal( 50 )
          expect( d.stats.in.count ).to.equal( 100 )
          expect( d.stats.in.skip ).to.equal( 50 )
          expect( d.stats.out.count ).to.equal( 50 )

          server.close()
          done()
        }
      } )

      expect( channel ).to.be.an( "object" )
      expect( channel.close ).to.be.an( "function" )
      expect( channel ).to.have.property( "port" ).that.is.a( "number" )
      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( var i = 0 ;  i < 50; i ++ ) {
        sendpk( i, i, channel.port, server, 25 )
      }

      for( ;  i < 100; i ++ ) {
        sendpk( i, i, channel.port, server, 77 )
      }

      setTimeout( () => channel.close(), 2100 )
    } )
  } )

  it( `create channel echo and close on timeout`, function( done ) {

    this.timeout( 21000 )
    this.slow( 20000 )

    let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": 20765, "codec": 0 } }, function( d ) {

      if( "close" === d.action ) {

        expect( d.stats.in.count ).to.equal( 0 )
        expect( d.stats.in.skip ).to.equal( 0 )
        expect( d.stats.out.count ).to.equal( 0 )

        done()
      }
    } )
  } )

  it( `check channel stats at the end`, function( done ) {

    let s = projectrtp.rtpchannel.stats()

    expect( s.available ).to.equal( 5000 )
    /* For now this might not be zero as the destructor is
    call when the JS object is cleaned up */
    //expect( s.current ).to.equal( 0 )
    done()
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
