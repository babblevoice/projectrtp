

const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp
const dgram = require( "dgram" )
const { channel } = require("diagnostics_channel")


function sendpk( sn, sendtime, dstport, server, ssrc = 25, pklength = 172 ) {

  setTimeout( () => {
    let payload = Buffer.alloc( pklength - 12 ).fill( sn & 0xff )
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


describe( "autocorrect", function() {

  it( `autocorrect by rtp packet`, async function() {

    this.timeout( 3000 )
    this.slow( 2500 )

    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    let onserverresolv
    let onserver = new Promise( ( r ) => onserverresolv = r )

    server.bind()
    server.on( "listening", async function() {
      onserverresolv()
    } )

    await onserver

    let channelcloseresolv
    let channelclose = new Promise( ( r ) => channelcloseresolv = r )
    let channelaclosestats
    const channela = await projectrtp.openchannel( {}, ( d ) => {
      if( "close" == d.action ) {
        channelaclosestats = d
        channelcloseresolv()
      }
    } )

    // Send somewhere bizarre to check auto correct!
    channela.remote( { "address":"172.65.33.2","port":45663 } )
    channela.echo()


    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i, channela.local.port, server )
    }

    setTimeout( () => channela.close(), 2000 )
    
    await channelclose

    server.close()
    expect( receviedpkcount ).to.equal( 50 )

  } )

} )

