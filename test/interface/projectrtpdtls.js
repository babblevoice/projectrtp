/*
Currently we use ourselelves to test us working - which is not ideal but better than nothing.

Ref: https://github.com/nplab/DTLS-Examples/blob/master/DTLS.pdf


Client                              Server            Flight
  |-------Client Hello--------------->| (use_srtp)      1
  |<------Hello Verify Request--------| (optional)      2
  |-------Client Hello--------------->| (optional)      3
  |<------Server Hello----------------| (use_srtp)      4
  |<------Server Certificate----------| (optional)      4
  |<------Certificate Request---------| (optional)      4
  |<------Server Key Exchange---------| (optional)      4
  |<------Server hello done-----------|                 4
  |-------Client Certificate--------->| (optional)      5
  |-------Client Key Exchange-------->|                 5
  |-------Certificate Verify--------->| (optional)      5
  |-------Change Cipher spec--------->|                 5
  |-------Finished------------------->|                 5
  |<------Change Cipher spec----------|                 6
  |<------Finished--------------------|                 6

  RFC also makes note of this diagram:

           Client                                               Server

         ClientHello                  -------->
                                                         ServerHello
                                                        Certificate*
                                                  ServerKeyExchange*
                                                 CertificateRequest*
                                      <--------      ServerHelloDone
         Certificate*
         ClientKeyExchange
         CertificateVerify*
         [ChangeCipherSpec]
         Finished                     -------->
                                                  [ChangeCipherSpec]
                                      <--------             Finished
         Application Data             <------->     Application Data

  Items which are optional in the DTLS are marked with *

  As UDP is unreliable then timeouts can occur. Each message is not timed out and resent - each flight is
  so if something is missing then the whole flight will be retranmitted.

  RFC 5764 requirements:
  The client must use use_srtp extension
  If supported, the server must respond in its hello with a use_srtp extension
  The client *must* offer a protection profile
  The server *must* offer a protection profile which has been offered by the client
  If there is no shared profile the use_srtp extension should not be added

  The srtp_mki value MAY... (only may so I won't look for issues there yet)

    
*/
const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp
const fs = require( "fs" )

/* Tests */
describe( "dtls", function() {

  it( "Test we have a fingerprint global", async function() {
    expect( projectrtp.dtls.fingerprint ).to.be.a( "string" )
    expect( projectrtp.dtls.fingerprint.length ).to.equal( 95 )
  } )

  it( "Test we have a fingerprint in channel", async function() {

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channel = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) done()
    } )

    expect( channel.local.dtls.fingerprint ).to.be.a( "string" )
    expect( channel.local.dtls.fingerprint.length ).to.equal( 95 )

    expect( channel.local.dtls.enabled ).to.be.a( "boolean" )
    expect( channel.local.dtls.enabled ).to.equal( false )

    channel.close()
    await finished
  } )

  it( "Create 2 channels and negotiate dtls", async function() {

    this.timeout( 6000 )
    this.slow( 2500 )

    projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "/tmp/ukringing.wav" )

    const targeta = {
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

    const targetb = {
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
    const finished = new Promise( ( r ) => { done = r } )

    let channelaclose = {}
    const channela = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelaclose = d
        channelb.close()
      }
    } )

    let channelbclose = {}
    const channelb = await projectrtp.openchannel( {}, function( d ) {
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

  it( "Create 2 channels and call remote", async function() {

    /*
                    |     internal projectrtp      |
    Clienta         |  channela          channelb  |        clientb
        |  RTP (DTLS)     |       MIX       |      RTP        |
        |<--------------->|<--------------->|<--------------->|
        play                                                 echo

    Once confiured, call remote on clienta again to try and break
    (simulate the current issue with 183).

    ussually - when run standalone (not garanteed):
    clienta port: 10002
    channela port: 10000
    channelb port: 10004
    clientb port: 10006
    */

    this.timeout( 6000 )
    this.slow( 2500 )

    projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "/tmp/ukringing.wav" )

    const channeltargeta = {
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

    const clienttargeta = {
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

    const channeltargetb = {
      "address": "localhost",
      "port": 0,
      "codec": 0
    }

    const clienttargetb = {
      "address": "localhost",
      "port": 12010,
      "codec": 0
    }

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelb.close()
      }
    } )

    let clientaclose = {}
    const clienta = await projectrtp.openchannel( {}, function( d ) {
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

    const channelb = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        clienta.close()
      }
    } )

    const clientb = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
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

  it( "Connect then remote to another session", async function() {

    /*
      Play, then encrypt at one end, then unencrypt in projectrtp and send onto d
      Create an e channel so that we mix 3

                                      |      internal       |
      channel                        channel          channel                   channel
        a       RTP over DTLS -----------b               c      RTP               d
        play                             b       mix     c                      echo
                                                 mix     e                         f
                                                                                echo
    
    */

    this.timeout( 6000 )
    this.slow( 2500 )

    projectrtp.tone.generate( "400+450*0.25/0/400+450*0.25/0:400/200/400/2000", "/tmp/ukringing.wav" )

    const targeta = {
      "address": "localhost",
      "port": 0,
      "codec": 0,
      "dtls": {
        "fingerprint": {
          "hash": ""
        },
        "mode": "active"
      }
    }

    const targetb = JSON.parse( JSON.stringify( targeta ) )

    const targetc =     {
      "address": "localhost",
      "port": 0,
      "codec": 0
    }
    const targetd = JSON.parse( JSON.stringify( targetc ) )
    const targete = JSON.parse( JSON.stringify( targetc ) )
    const targetf = JSON.parse( JSON.stringify( targetc ) )

    let done
    const finished = new Promise( ( resolve ) => { done = resolve } )

    const channela = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelb.close()
      }
    } )

    const channelb = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelc.close()
      }
    } )

    const channelc = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channeld.close()
      }
    } )

    const channeld = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channele.close()
      }
    } )

    const channele = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelf.close()
      }
    } )

    let channelclose = {}
    const channelf = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelclose = d
        done()
      }
    } )

    targeta.dtls.fingerprint.hash = channelb.local.dtls.fingerprint
    targeta.port = channelb.local.port
    expect( channela.remote( targeta ) ).to.be.true
    targetb.port = channela.local.port
    targetb.dtls.fingerprint.hash = channela.local.dtls.fingerprint
    targetb.dtls.mode = "passive"
    expect( channelb.remote( targetb ) ).to.be.true

    targetc.port = channeld.local.port
    expect( channelc.remote( targetc ) ).to.be.true
    targetd.port = channelc.local.port
    expect( channeld.remote( targetd ) ).to.be.true

    targete.port = channelf.local.port
    expect( channele.remote( targete ) ).to.be.true
    targetf.port = channele.local.port
    expect( channelf.remote( targetf ) ).to.be.true

    /* play on one end */
    channela.play( { "loop": true, "files": [
      { "wav": "/tmp/ukringing.wav" } ] } )

    /* mix in the middle */
    channelb.mix( channelc )
    channelb.mix( channele )

    /* echo at the other end */
    expect( channeld.echo() ).to.be.true

    await new Promise( ( r ) => { setTimeout( () => r(), 1500 ) } )

    channela.close()

    await fs.promises.unlink( "/tmp/ukringing.wav" ).catch( () => {} )
    await finished

    expect( channelclose.stats.in.count ).to.be.above( 30 )
    expect( channelclose.stats.in.skip ).to.be.below( 2 ) // allow a little loss in test

  } )
} )
