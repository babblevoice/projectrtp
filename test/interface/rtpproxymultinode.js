

const expect = require( "chai" ).expect
const prtp = require( "../../index.js" ).projectrtp
const mocknode = require( "../mock/mocknode" )

describe( "rtpproxy multi node", function() {

  it( `2 node simple mix`, async function() {
    /*
      SIP                   rtp 1                          rtp 2
      open ------------------>                                        (1)
           <-----------------id:1                                     (2)
      open -------------------------------------------------->        (3)
           <--------------------------------------------------id:2    (4)

      mix converts to:
           open channel ----->                                        (5)
           <------------------id:3                                    (6)

           open channel ------------------------------------->        (7)
           <-------------------------------------------------id:4     (8)

           remote ----------->                                        (9)
           remote ------------------------------------------->        (10)

           mix 1,3 ---------->                                        (11)
           mix 2,4 ------------------------------------------>        (12)

      unmix (on both channels) converts to:
           unmix 1 ---------->                                        (13)
           unmix 3 ---------->                                        (14)

           unmix 2 ------------------------------------------>        (15)
           unmix 4 ------------------------------------------>        (16)

           close channel 3 -->                                        (17)
           close channel 4 ---------------------------------->        (18)
    */
    let rtp1 = new mocknode()
    let rtp2 = new mocknode()

    const listenport = 9008

    let rtpreceveivedmessages = []
    let ouruuid = 0

    rtp1.setmessagehandler( "open", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
      rtp1.sendmessage( {
          "action": "open",
          "id": msg.id,
          "uuid": ""+ouruuid++,
          "local": {
            "port": 10002,
            "address": "192.168.0.141"
            },
          "status": rtp1.ourstats
          } )
    } )

    rtp1.setmessagehandler( "mix", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "mix" )
      expect( msg ).to.have.property( "other" ).that.is.a( "object" )
      expect( msg.other ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg.other ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
    } )

    rtp1.setmessagehandler( "unmix", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
    } )

    rtp1.setmessagehandler( "remote", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "remote" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "remote" )
    } )

    rtp1.setmessagehandler( "close", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
      rtp1.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid,
        } )
    } )

    rtp2.setmessagehandler( "open", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
      rtp2.sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": ""+ouruuid++,
        "local": {
          "port": 10004,
          "address": "192.168.0.141"
          },
        "status": rtp2.ourstats
        } )
    } )

    rtp2.setmessagehandler( "mix", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "mix" )
      expect( msg ).to.have.property( "other" ).that.is.a( "object" )
      expect( msg.other ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg.other ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
    } )

    rtp2.setmessagehandler( "unmix", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
    } )

    rtp2.setmessagehandler( "remote", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "remote" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "remote" )
    } )

    rtp2.setmessagehandler( "close", ( msg ) => {  
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )

      rtp2.sendmessage( {
        "action": "close",
        "uuid": msg.uuid,
        "id": msg.id
      } )
    } )

    let p = await prtp.proxy.listen( undefined, "127.0.0.1", listenport )
    rtp1.connect( listenport )
    rtp2.connect( listenport )
    await p.waitfornewconnection()
    
    let channel1 = await prtp.openchannel( { "nodeinstance": rtp1.id } )
    let channel2 = await prtp.openchannel( { "nodeinstance": rtp2.id } )
    await channel1.mix( channel2 )
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1000 ) } )
    channel1.unmix()
    channel2.unmix()

    channel1.close()
    channel2.close()

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 10 ) } )

    console.log(rtpreceveivedmessages)
    /* Steps 1-4 */
    expect( rtpreceveivedmessages[ 0 ].channel ).to.equal( "open" )
    expect( rtpreceveivedmessages[ 0 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 1 ].channel ).to.equal( "open" )
    expect( rtpreceveivedmessages[ 1 ].node ).to.equal( "rtp2" )

    /* Steps 5-8 */
    expect( rtpreceveivedmessages[ 2 ].channel ).to.equal( "open" )
    expect( rtpreceveivedmessages[ 2 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 3 ].channel ).to.equal( "open" )
    expect( rtpreceveivedmessages[ 3 ].node ).to.equal( "rtp2" )

    /* Steps 9, 10 */
    expect( rtpreceveivedmessages[ 4 ].channel ).to.equal( "remote" )
    expect( rtpreceveivedmessages[ 4 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 4 ].remote.port ).to.be.an( "number" )
    expect( rtpreceveivedmessages[ 4 ].remote.address ).to.be.an( "string" )
    expect( rtpreceveivedmessages[ 6 ].channel ).to.equal( "remote" )
    expect( rtpreceveivedmessages[ 6 ].node ).to.equal( "rtp2" )
    expect( rtpreceveivedmessages[ 6 ].remote.port ).to.be.an( "number" )
    expect( rtpreceveivedmessages[ 6 ].remote.address ).to.be.an( "string" )


    /* Steps 11, 12 */
    expect( rtpreceveivedmessages[ 5 ].channel ).to.equal( "mix" )
    expect( rtpreceveivedmessages[ 5 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 7 ].channel ).to.equal( "mix" )
    expect( rtpreceveivedmessages[ 7 ].node ).to.equal( "rtp2" )

    /* Steps 13, 14*/
    expect( rtpreceveivedmessages[ 8 ].channel ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 8 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 8 ] ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 8 ] ).to.have.property( "id" ).that.is.a( "string" )
    expect( rtpreceveivedmessages[ 8 ] ).to.have.property( "uuid" ).that.is.a( "string" )
    expect( rtpreceveivedmessages[ 10 ].channel ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 10 ].node ).to.equal( "rtp2" )
    expect( rtpreceveivedmessages[ 10 ] ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 10 ] ).to.have.property( "id" ).that.is.a( "string" )
    expect( rtpreceveivedmessages[ 10 ] ).to.have.property( "uuid" ).that.is.a( "string" )

    /* Steps 17, 18 */
    expect( rtpreceveivedmessages[ 9 ].channel ).to.equal( "close" )
    expect( rtpreceveivedmessages[ 9 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 11 ].channel ).to.equal( "close" )
    expect( rtpreceveivedmessages[ 11 ].node ).to.equal( "rtp2" )

    /* Clean up */
    rtp1.destroy()
    rtp2.destroy()
    p.destroy()

  } )

  it( `2 node 1 channel on one, 2 channels other`, async function() {

  } )

  it( `2 node 2 channels on one, 2 channels other`, async function() {

  } )

  it( `2 channels with existing nodebridges array which need to be combined`, async function() {

  } )
} )