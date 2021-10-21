
const expect = require( "chai" ).expect
const prtp = require( "../../index.js" ).projectrtp
const mocknode = require( "./mocknode.js" )

describe( "rtpnode", function() {
  it( `rtpnode check open json`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
          "action": "open",
          "id": onmsg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    n.setmessagehandler( "close", ( onmsg ) => {
      p.close()
      n.destroy()
    } )

    let p = await prtp.proxy.listen( 45000, "127.0.0.1" )
    n.connect()
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()

    expect( channel ).to.be.an( "object" )
    expect( channel.close ).to.be.an( "function" )
    expect( channel.mix ).to.be.an( "function" )
    expect( channel.unmix ).to.be.an( "function" )
    expect( channel.echo ).to.be.an( "function" )
    expect( channel.play ).to.be.an( "function" )
    expect( channel.record ).to.be.an( "function" )
    expect( channel.direction ).to.be.an( "function" )
    expect( channel.dtmf ).to.be.an( "function" )
    expect( channel.target ).to.be.an( "function" )
    expect( channel.local ).to.have.property( "port" ).that.is.a( "number" )
    expect( channel.local ).to.have.property( "address" ).that.is.a( "string" )
    expect( channel.local.port ).to.equal( 10002 )
    expect( channel.local.address ).to.equal( "192.168.0.141" )
    expect( channel.uuid ).that.is.a( "string" )
    expect( channel.id ).that.is.a( "string" )

    channel.close()
  } )

  it( `rtpnode check echo`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
          "action": "open",
          "id": onmsg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    n.setmessagehandler( "echo", ( onmsg ) => {
      console.log( "receieved echo" )
      console.log(onmsg)
    } )

    n.setmessagehandler( "close", ( onmsg ) => {
      p.close()
      n.destroy()
    } )

    let p = await prtp.proxy.listen( 45000, "127.0.0.1" )
    n.connect()
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.echo()
    channel.close()

  } )

  it( `rtpnode check dtmf`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
          "action": "open",
          "id": onmsg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    n.setmessagehandler( "dtmf", ( onmsg ) => {
      console.log(onmsg)
    } )

    n.setmessagehandler( "close", ( onmsg ) => {
      p.close()
      n.destroy()
    } )

    let p = await prtp.proxy.listen( 45000, "127.0.0.1" )
    n.connect()
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.dtmf( "#123" )
    channel.close()

  } )

  it( `rtpnode check mix/unmix`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
          "action": "open",
          "id": msg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    let mixreceived = false
    let unmixreceived = false
    n.setmessagehandler( "mix", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "mix" )
      expect( msg ).to.have.property( "other" ).that.is.a( "string" ).to.equal( "otheruuid" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      mixreceived = true
    } )

    n.setmessagehandler( "unmix", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      unmixreceived = true
    } )

    n.setmessagehandler( "close", ( msg ) => {
      p.close()
      n.destroy()
    } )

    let p = await prtp.proxy.listen( 45000, "127.0.0.1" )
    n.connect()
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.mix( "otheruuid" )
    channel.unmix()

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 10 ) } )

    channel.close()

    expect( mixreceived ).to.be.true
    expect( unmixreceived ).to.be.true

  } )

  it( `rtpnode check target`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
          "action": "open",
          "id": msg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    let targetreceived = false
    n.setmessagehandler( "target", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "target" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "target" ).that.is.a( "string" ).to.equal( "wouldbeatargetobject" )

      targetreceived = true
    } )

    n.setmessagehandler( "close", ( msg ) => {
      p.close()
      n.destroy()
    } )

    let p = await prtp.proxy.listen( 45000, "127.0.0.1" )
    n.connect()
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.target( "wouldbeatargetobject" )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 10 ) } )

    channel.close()

    expect( targetreceived ).to.be.true

  } )


  it( `rtpnode check play/record`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
          "action": "open",
          "id": msg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    let playreceived = false
    let recordreceived = false
    n.setmessagehandler( "play", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "play" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "soup" ).that.is.a( "string" ).to.equal( "wouldbeaplayobject" )

      playreceived = true
    } )

    n.setmessagehandler( "record", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "record" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "options" ).that.is.a( "string" ).to.equal( "wouldbearecordobject" )

      recordreceived = true
    } )

    n.setmessagehandler( "close", ( msg ) => {
      p.close()
      n.destroy()
    } )

    let p = await prtp.proxy.listen( 45000, "127.0.0.1" )
    n.connect()
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.play( "wouldbeaplayobject" )
    channel.record( "wouldbearecordobject" )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 10 ) } )

    channel.close()

    expect( playreceived ).to.be.true
    expect( recordreceived ).to.be.true
  } )
} )
