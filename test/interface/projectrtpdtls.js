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

const { exec } = require( "node:child_process" )

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

  it( `Create 2 channels and negotiate dtls`, async function() {

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

    let clientb = await projectrtp.openchannel( {}, function( d ) {
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

  it( `Connect then remote to another session`, async function() {

    /*
                    |     internal projectrtp      |
    Clienta         |  channela          channelb  |        clientb
        |  RTP (DTLS)     |       MIX       |      RTP        |
        |<--------------->|<--------------->|<--------------->| Step 1.
        |<-(close)-(rem)->|<--------------->|<--------------->| Step 2.
    Clientc
        |<--------------->|<--------------->|<--------------->|Step 3.
        play                                                 echo

    
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

    let channeltargetc = {
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

    let channela = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelb.close()
      }
    } )

    let clientaclose
    let clienta = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        clientaclose = d
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


    let channelb = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        clientb.close()
      }
    } )

    let clientb = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        clientc.close()
      }
    } )

    let clientcclose
    let clientc = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        clientcclose = d
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

    channeltargetc.dtls.fingerprint.hash = clientc.local.dtls.fingerprint
    channeltargetc.port = clientc.local.port
    expect( channela.remote( channeltargetc ) ).to.be.true
    expect( clientc.remote( clienttargeta ) ).to.be.true

    clientc.play( { "loop": true, "files": [
      { "wav": "/tmp/ukringing.wav" } ] } )
    clienta.close()

    await new Promise( ( r ) => { setTimeout( () => r(), 1500 ) } )

    channela.close()

    await fs.promises.unlink( "/tmp/ukringing.wav" ).catch( () => {} )
    await finished

    expect( clientcclose.stats.in.count ).to.be.above( 10 )
    expect( clientcclose.stats.in.skip ).to.equal( 0 )


  } )


  it( `Create TLS UDP server`, async function() {
    
    /* only used to play with */
    if(1)return
    /*
    Use openssl to test our connection.
    openssl (client)               project (server)
       |                              |
       |-------client hello---------->| (use_dtls)
       |<------server hello-----------|
       |<------certificate------------|
       |<------server key exchange----|
       |<------cert request  (frag)---| (multiple)
       |<------server hello done------|
       |---cert client key exchange---|
       ...
    */

    this.timeout( 25000 )
    this.slow( 15000 )

    let channeltarget = {
      "address": "localhost",
      "port": 10002,
      "codec": 0,
      "dtls": {
        "fingerprint": {
          "hash": ""
        },
        "mode": "passive" // - is this in the right place and the right way round!
      }
    }

    let channel = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelclose = d
      }
    } )

    channeltarget.dtls.fingerprint.hash = channel.local.dtls.fingerprint
    channel.remote( channeltarget )

    await new Promise( ( r ) => { setTimeout( () => r(), 1000 ) } )

    let execcompleted
    let execwait = new Promise( ( r ) => execcompleted )

    exec( `openssl s_client -connect 127.0.0.1:10000 -cert ~/.projectrtp/certs/dtls-srtp.pem -noservername -brief -dtls -mtu 1452 -bind 127.0.0.1:10002`, ( error, stdout, stderr ) => {
      if (error) {
        console.error(`exec error: ${error}`);
        return;
      }
      console.log(`stdout: ${stdout}`);
      console.error(`stderr: ${stderr}`);

      execcompleted()
    } )

    console.log("hi")

    await execwait

    
  } )

  it( `Create TLS UDP client`, async function() {
    /* only used to play with */
    if(1)return
    /*
    Use openssl to test our connection.
    project (client)                                   openssl (server)
       |                                                  |
       |--------------client hello----------------------->|
       |<-------------hello verify request----------------|
       |--------------client hello----------------------->|
       |<--server hello, certificate, server key exchange-|
       |<--server key exchange(reas) server hello done----|
       |<-------------client key exchange-----------------| (one)
       |<-------------change cypher spec------------------|
       |<---------encryped handshake message--------------|
       |--------new session ticket----------------------->|
       ...
    */

    this.timeout( 25000 )
    this.slow( 15000 )

    let channeltarget = {
      "address": "localhost",
      "port": 10002,
      "codec": 0,
      "dtls": {
        "fingerprint": {
          "hash": ""
        },
        "mode": "active" // - is this in the right place and the right way round!
      }
    }

    let channel = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        channelclose = d
      }
    } )

    channeltarget.dtls.fingerprint.hash = channel.local.dtls.fingerprint
    channel.remote( channeltarget )

    await new Promise( ( r ) => { setTimeout( () => r(), 1000 ) } )

    let execcompleted
    let execwait = new Promise( ( r ) => execcompleted )

    exec( `openssl s_server -cert ~/.projectrtp/certs/dtls-srtp.pem -brief -dtls1_2 -use_srtp SRTP_AES128_CM_SHA1_80 -mtu 1452 -port 10002`, ( error, stdout, stderr ) => {
      if (error) {
        console.error(`exec error: ${error}`);
        return;
      }
      console.log(`stdout: ${stdout}`);
      console.error(`stderr: ${stderr}`);

      execcompleted()
    } )

    console.log("hi")

    await execwait

  } )
} )
