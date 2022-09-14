/*
Currently we use ourselelves to test us working - which is not ideal but better than nothing.
*/
const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp
const fs = require( "fs" )

/* Tests */
describe( "dtls", function() {

  it( `Test we have a fingerprint global`, async function() {
    expect( projectrtp.dtls.fingerprint ).to.be.a( "string" )
    expect( projectrtp.dtls.fingerprint.length ).to.equal( 95 )
  } )

  it( `Test we have a fingerprint in channel`, async function() {

    let done
    let finished = new Promise( ( r ) => { done = r } )

    let channel = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) done()
    } )

    expect( channel.local.dtls.fingerprint ).to.be.a( "string" )
    expect( channel.local.dtls.fingerprint.length ).to.equal( 95 )

    expect( channel.local.dtls.enabled ).to.be.a( "boolean" )
    expect( channel.local.dtls.enabled ).to.equal( false )

    channel.close()
    await finished
  } )

  it( `Create 2 channels and negotiate`, async function() {

    this.timeout( 6000 )
    this.slow( 2500 )

    projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "/tmp/ukringing.wav" )

    let targeta = {
      "address": "localhost",
      "port": 0,
      "codec": 0,
      "dtls": {
        "fingerprint": {
          "hash": ""
        },
        "mode": "active" // - is this in the right place and the right way round!
      }
    }

    let targetb = {
      "address": "localhost",
      "port": 12008,
      "codec": 0,
      "dtls": {
        "fingerprint": {
          "hash": projectrtp.dtls.fingerprint
        },
        "mode": "passive"
      }
    }

    let done
    let finished = new Promise( ( r ) => { done = r } )

    let channelaclose
    let channela = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelaclose = d
        channelb.close()
      }
    } )

    let channelbclose
    let channelb = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelbclose = d
        done()
      }
    } )

    targeta.dtls.fingerprint.hash = channelb.local.dtls.fingerprint
    targeta.port = channelb.local.port

    expect( channela.remote( targeta ) ).to.be.true

    targetb.port = channela.local.port

    expect( channelb.remote( targetb ) ).to.be.true

    channela.play( { "loop": true, "files": [
                      { "wav": "/tmp/ukringing.wav" } ] } )

    expect( channelb.echo() ).to.be.true

    await new Promise( ( r ) => { setTimeout( () => r(), 2000 ) } )

    channela.close()

    await fs.promises.unlink( "/tmp/ukringing.wav" ).catch( () => {} )
    await finished

    expect( channelaclose.reason ).to.equal( "requested" )
    expect( channelaclose.stats.in.count ).to.be.above( 70 )
    expect( channelaclose.stats.in.skip ).to.equal( 0 )

    expect( channelbclose.reason ).to.equal( "requested" )
    expect( channelbclose.stats.in.count ).to.be.above( 70 )
    expect( channelbclose.stats.in.skip ).to.equal( 0 )

  } )

  it( `Create 2 channels and call remote`, async function() {

    /*
                    |     internal projectrtp      |
    Clienta         |  channela          channelb  |        clientb
        |  RTP (DTLS)     |       MIX       |      RTP        |
        |<--------------->|<--------------->|<--------------->|
        play                                                 echo

    Once confiured, call remote on clienta again to try and break
    (simulate the current issue with 183).
    */

    this.timeout( 6000 )
    this.slow( 2500 )

    projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "/tmp/ukringing.wav" )

    let channeltargeta = {
      "address": "localhost",
      "port": 0,
      "codec": 0,
      "dtls": {
        "fingerprint": {
          "hash": ""
        },
        "mode": "active" // - is this in the right place and the right way round!
      }
    }

    let clienttargeta = {
      "address": "localhost",
      "port": 12008,
      "codec": 0,
      "dtls": {
        "fingerprint": {
          "hash": ""
        },
        "mode": "passive"
      }
    }

    let channeltargetb = {
      "address": "localhost",
      "port": 0,
      "codec": 0
    }

    let clienttargetb = {
      "address": "localhost",
      "port": 12010,
      "codec": 0
    }

    let done
    let finished = new Promise( ( r ) => { done = r } )

    let channelaclose
    let channela = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelaclose = d
        channelb.close()
      }
    } )

    let clientaclose
    let clienta = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        clientaclose = d
        clientb.close()
      }
    } )

    channeltargeta.dtls.fingerprint.hash = clienta.local.dtls.fingerprint
    channeltargeta.port = clienta.local.port
    expect( channela.remote( channeltargeta ) ).to.be.true
    clienttargeta.port = channela.local.port
    clienttargeta.dtls.fingerprint.hash = channela.local.dtls.fingerprint
    expect( clienta.remote( clienttargeta ) ).to.be.true

    clienta.play( { "loop": true, "files": [
                    { "wav": "/tmp/ukringing.wav" } ] } )

    let channelbclose
    let channelb = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelbclose = d
        clienta.close()
      }
    } )

    let clientbclose
    let clientb = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        clientbclose = d
        done()
      }
    } )

    channeltargetb.port = clientb.local.port
    expect( channelb.remote( channeltargetb ) ).to.be.true
    clienttargetb.port = channelb.local.port
    expect( clientb.remote( clienttargetb ) ).to.be.true
    expect( clientb.echo() ).to.be.true

    channela.mix( channelb )

    await new Promise( ( r ) => { setTimeout( () => r(), 500 ) } )

    channeltargeta.codec = 9
    expect( channela.remote( channeltargeta ) ).to.be.true
    expect( channelb.remote( channeltargetb ) ).to.be.true
    channela.mix( channelb )

    await new Promise( ( r ) => { setTimeout( () => r(), 2500 ) } )

    channela.close()

    await fs.promises.unlink( "/tmp/ukringing.wav" ).catch( () => {} )
    await finished

    expect( clientaclose.reason ).to.equal( "requested" )
    expect( clientaclose.stats.in.count ).to.be.above( 70 )
    expect( clientaclose.stats.in.skip ).to.equal( 0 )

  } )
} )
