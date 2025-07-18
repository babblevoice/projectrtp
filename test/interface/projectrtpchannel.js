
const expect = require( "chai" ).expect
const dgram = require( "dgram" )

const projectrtp = require( "../../index" ).projectrtp
const node = require( "../../lib/node" ).interface
const server = require( "../../lib/server" ).interface
const rtputil = require( "../util/rtp" )

/**
 * Common channel tester
 * @param { object } channel 
 * @param { string } id 
 */
function exepectchannel( channel, id ) {

  expect( channel ).to.be.an( "object" )
  expect( channel.close ).to.be.an( "function" )
  expect( channel.mix ).to.be.an( "function" )
  expect( channel.unmix ).to.be.an( "function" )
  expect( channel.echo ).to.be.an( "function" )
  expect( channel.play ).to.be.an( "function" )
  expect( channel.record ).to.be.an( "function" )
  expect( channel.direction ).to.be.an( "function" )
  expect( channel.dtmf ).to.be.an( "function" )
  expect( channel.remote ).to.be.an( "function" )
  expect( channel.local ).to.have.property( "port" ).that.is.a( "number" )
  expect( channel.local ).to.have.property( "address" ).that.is.a( "string" )
  expect( channel.local ).to.have.property( "icepwd" ).that.is.a( "string" ).to.have.lengthOf.above( 20 )
  expect( channel.uuid ).that.is.a( "string" )
  expect( channel.id ).that.is.a( "string" )
  expect( channel.id ).to.equal( id )
}

/**
 * @ignore
 * @param { number } sn 
 * @param { number } sendtime 
 * @param { number } dstport 
 * @param { object } server 
 * @param { number } ssrc 
 * @param { number } pklength 
 */
function sendpk( sn, sendtime, dstport, server, ssrc = 25, pklength = 172 ) {

  setTimeout( () => {
    const payload = Buffer.alloc( pklength - 12 ).fill( sn & 0xff )
    const ts = sn * 160
    const tsparts = []
    /* portability? */
    tsparts[ 3 ] = ( ts & 0xff000000 ) >> 24
    tsparts[ 2 ] = ( ts & 0xff0000 ) >> 16
    tsparts[ 1 ] = ( ts & 0xff00 ) >> 8
    tsparts[ 0 ] = ( ts & 0xff )

    const snparts = []
    sn = ( sn + 100 ) % ( 2**16 ) /* just some offset */
    snparts[ 0 ] = sn & 0xff
    snparts[ 1 ] = sn >> 8

    const ssrcparts = []
    ssrcparts[ 3 ] = ( ssrc & 0xff000000 ) >> 24
    ssrcparts[ 2 ] = ( ssrc & 0xff0000 ) >> 16
    ssrcparts[ 1 ] = ( ssrc & 0xff00 ) >> 8
    ssrcparts[ 0 ] = ( ssrc & 0xff )


    const rtppacket = Buffer.concat( [
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

  it( "call create channel and check the structure of the returned object", async function() {

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": 20000, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    exepectchannel( channel, "4" )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    channel.close()
    await finished
  } )

  it( "call create channel and check the structure of the returned object - server as listener", async function() {

    const ourport = 45433
    await projectrtp.server.listen( ourport, "127.0.0.1" )
    const ournode = await projectrtp.node.connect( ourport, "127.0.0.1" )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": 20000, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    exepectchannel( channel, "4" )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    channel.close()
    await finished

    ournode.destroy()
    await server.destroy()
  } )

  it( "call create channel and check the structure of the returned object - node as listener", async function() {

    const ourport = 45432
    projectrtp.server.clearnodes()
    projectrtp.server.addnode( { host: "127.0.0.1", port: ourport } )

    const ournode = node.create( projectrtp )
    const n = await ournode.listen( "127.0.0.1", ourport )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": 20000, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    exepectchannel( channel, "4" )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    channel.close()
    await finished

    projectrtp.server.clearnodes()
    n.destroy()
  } )

  it( "call create channel echo", function( done ) {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0
    server.on( "message", function() {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        expect( receivedChunks.length ).to.be.greaterThan( 0 )
        expect( receivedChunks[ 0 ] ).to.be.instanceOf( Buffer )
        expect( receivedChunks[ 0 ].length ).to.be.greaterThan( 0 )

        if( "close" === d.action ) {
          expect( d.reason ).to.equal( "requested" )
          expect( receviedpkcount ).to.equal( 50 )
          expect( d.stats.in.count ).to.equal( 50 )
          expect( d.stats.in.mos ).to.equal( 4.5 )
          expect( d.stats.in.dropped ).to.equal( 0 )
          expect( d.stats.in.skip ).to.equal( 0 )
          expect( d.stats.out.count ).to.equal( 50 )

          server.close()
          done()
        }
      } )

      const rs = await channel.readstream();

      const receivedChunks = [];

      rs.on( "data", ( chunk ) => {
        receivedChunks.push( chunk )
      } )

      expect( channel ).to.be.an( "object" )
      expect( channel.close ).to.be.an( "function" )
      expect( channel.local ).to.have.property( "port" ).that.is.a( "number" )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0;  50 > i; i ++ ) {
        sendpk( i, i, channel.local.port, server )
      }

      setTimeout( () => channel.close(), 2000 )
    } )
  } )

  it( "create channel echo and skip some packets", function( done ) {

    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0
    server.on( "message", function() {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {
          expect( receviedpkcount ).to.equal( 50 - 6 )
          expect( d.stats.in.count ).to.equal( 50 - 6 )
          expect( d.stats.out.count ).to.equal( 50 - 6 )

          server.close()
          done()
        }
      } )
      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0;  50 > i; i ++ ) {
        if( i in { 3:0, 13:0, 23:0, 24:0, 30:0, 49:0 } ) continue
        sendpk( i, i, channel.local.port, server )
      }

      setTimeout( () => channel.close(), 2000 )
    } )
  } )

  it( "create channel echo and send out of order packets", function( done ) {
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0

    let lastsn = -1
    let lastts = -1
    let totalsndiff = 0
    let totaltsdiff = 0
    server.on( "message", function( msg ) {
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
    server.on( "listening", async function() {

      const ourport = server.address().port

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

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

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      const sns = [ 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
        11, 13, 14, 12, 15, 16, 17, 18, 19, 20,
        21, 22, 23, 25, 24, 26, 27, 28, 29, 30,
        31, 37, 33, 34, 36, 35, 32, 38, 39, 40,
        41, 42, 43, 44, 45, 46, 47, 48, 49 ]

      sns.forEach( function( e, i ) {
        sendpk( e, i, channel.local.port, server )
      } )

      setTimeout( () => channel.close(), 2000 )
    } )
  } )

  it( "create channel echo and send packets outside of window", function( done ) {
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0
    server.on( "message", function() {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {

          expect( receviedpkcount ).to.equal( 47 )
          expect( d.stats.in.count ).to.equal( 50 )
          expect( d.stats.in.dropped ).to.equal( 3 )
          expect( d.stats.out.count ).to.equal( 47 )

          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      const sns = [ 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
        11, 13, 14, 12, 15, 16, 17, 18, 19, 20,
        21, 22, 23, 25, 24, 26, 27, 28, 29, 100,
        31, 37, 33, 34, 36, 35, 32, 38, 39, 400,
        41, 42, 43, 44, 45, 46, 47, 48, 2 ]

      sns.forEach( function( e, i ) {
        sendpk( e, i, channel.local.port, server )
      } )

      setTimeout( () => channel.close(), 2000 )
    } )
  } )

  it( "create channel echo and simulate a stalled connection", async function() {
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0

    let channel

    let firstsn = 0
    let lastsn = -1
    let lastts = -1
    let totalsndiff = 0
    let totaltsdiff = 0
    server.on( "message", function( msg ) {
      const pk = rtputil.parsepk( msg )
      const sn = pk.sn
      const ts = pk.ts

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

    this.timeout( 15000 )
    this.slow( 8000 )

    server.bind()


    let closedstats = {}
    server.on( "listening", async function() {

      const ourport = server.address().port

      channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {
          closedstats = d
          server.close()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      let i
      for( i = 0; 120 > i; i++ ) {
        sendpk( i, i, channel.local.port, server )
      }

      /* pause then catchup */
      for( ; 150 > i; i++ ) {
        sendpk( i, 150, channel.local.port, server )
      }

      /* resume */
      for( ; 300 > i; i++ ) {
        sendpk( i, i, channel.local.port, server )
      }
    } )

    await new Promise( resolve => setTimeout( resolve, 6500 ) )
    // @ts-ignore
    channel.close()
    await new Promise( resolve => setTimeout( resolve, 50 ) )

    /*
      We should receive
      50 from the first batch
      30 from the catchup as some will be dropped
      100 from the final batch as it will take some to get going again
    */
    expect( receviedpkcount ).to.equal( closedstats.stats.out.count )
    expect( closedstats.stats.in.count ).to.equal( 300 )
    expect( closedstats.stats.out.count ).to.be.above( 250 )
    expect( totalsndiff ).to.equal( 0 ) // received should be reordered
    expect( totaltsdiff ).to.be.within( 5000, 18400 ) // Allow some loss in test
    expect( lastsn - firstsn ).to.be.within( 250, 300 )
  } )

  it( "create channel echo whilst wrapping the sn", function( done ) {
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0
    server.on( "message", function() {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {

          expect( receviedpkcount ).to.equal( 50 )
          expect( d.stats.in.count ).to.equal( 50 )
          expect( d.stats.out.count ).to.equal( 50 )

          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0 ;  50 > i; i ++ ) {
        const sn = i + ( 2**16 ) - 25
        sendpk( sn, i, channel.local.port, server )
      }

      setTimeout( () => channel.close(), 2000 )
    } )
  } )


  it( "create channel echo and incorrectly change the ssrc", function( done ) {
    /* This needs further work so make work for now. Remove tests which check for ignored packets. */
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0
    server.on( "message", function() {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {

          expect( d.stats.in.count ).to.equal( 100 )
          expect( d.stats.out.count ).to.equal( receviedpkcount )

          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      let i
      for( i = 0 ;  50 > i; i ++ ) {
        sendpk( i, i, channel.local.port, server, 25 )
      }

      for( ;  100 > i; i ++ ) {
        sendpk( i, i, channel.local.port, server, 77 )
      }

      setTimeout( () => channel.close(), 2100 )
    } )
  } )

  it( "send oversized rtp packet", function( done ) {
    /* This test has been adjusted as we now handle large packets - but we should crash! */
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {
          expect( d.stats.in.count ).to.equal( 50 )
          expect( d.stats.out.count ).to.equal( 50 )

          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0 ;  50 > i; i ++ ) {
        if( 40 == i ) {
          /* an oversized packet */
          sendpk( i, i, channel.local.port, server, 25, 1200 )
        } else {
          sendpk( i, i, channel.local.port, server, 25 )
        }
      }

      setTimeout( () => channel.close(), 2100 )
    } )
  } )

  it( "create channel echo and close on timeout", function( done ) {

    this.timeout( 21000 )
    this.slow( 20000 )

    projectrtp.openchannel( { "remote": { "address": "localhost", "port": 20765, "codec": 0 } }, function( d ) {

      if( "close" === d.action ) {

        expect( d.stats.in.count ).to.equal( 0 )
        expect( d.stats.in.skip ).to.equal( 0 )
        expect( d.stats.out.count ).to.equal( 0 )

        done()
      }
    } )
  } )

  it( "create channel and check event emitter", async () => {

    this.timeout( 2000 )
    this.slow( 2000 )

    let done
    const waituntildone = new Promise( ( r ) => done = r )

    const chan = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": 20765, "codec": 0 } } )
    chan.em.on( "all", ( d ) => {
      if( "close" === d.action ) {

        expect( d.stats.in.count ).to.equal( 0 )
        expect( d.stats.in.skip ).to.equal( 0 )
        expect( d.stats.out.count ).to.equal( 0 )
        done()
      }
    } )

    chan.close()
    await waituntildone
  } )
} )
