// @ts-nocheck
// no check as this file is mainly here for documentation - and only a simple test is included.

const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp

const fs = require( "node:fs" ).promises
const dgram = require( "dgram" )
const rtp = require( "../util/rtp" )

describe( "tonegen", function() {

  const silentfilename = "/tmp/silent.wav"

  this.afterEach( async () => {
    try{
      await fs.unlink( silentfilename )
    } catch( e ) { /* silent */ }
    
  } )

  it( "tone.generate exists", async function() {
    expect( projectrtp.tone.generate ).to.be.an( "function" )
  } )

  it( "generate silence and ensure looped is silent", async function() {

    this.timeout( 4000 )
    this.slow( 3000 )

    /* Trying to replicate a bug where we use the silence in a tone file to send silence */
    projectrtp.tone.generate( "350+440*0.5:1000", silentfilename )
    projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", silentfilename )
    projectrtp.tone.generate( "697+1209*0.5/0/697+1336*0.5/0/697+1477*0.5/0/697+1633*0.5/0:400/100", silentfilename )

    const server = dgram.createSocket( "udp4" )

    let done
    const complete = new Promise( resolve => done = resolve )
    let recvcount = 0
    let allaregood = true
    server.on( "message", function( dgm ) {
      // check
      const parsed = rtp.parsepk( dgm )

      recvcount++

      allaregood &&= Array.from( parsed.payload ).every( val => 255 === val )

      if( 100 < recvcount ) channel.close()
    } )

    server.bind()
    await new Promise( resolve => server.on( "listening", resolve() ) )

    const ourport = server.address().port

    const chandef = { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }
    const channel = await projectrtp.openchannel( chandef, function( d ) {
      if( "close" === d.action ) {
        server.close()
        done()
      }
    } )

    const prompt = { "wav": silentfilename, "start": 2100, "stop": 3100, "loop": 3 } //silent
    const soup = { 
      "files": [ prompt ], "interupt": true 
    }

    channel.play( soup )

    await complete

    expect( allaregood ).to.be.true

  } )
} )

