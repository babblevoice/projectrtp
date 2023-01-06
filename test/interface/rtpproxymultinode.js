

const expect = require( "chai" ).expect
const prtp = require( "../../index.js" ).projectrtp
const mocknode = require( "../mock/mocknode" )

describe( "rtpproxy multi node", function() {

  it( "2 node simple mix", async function() {
    
    const actual = { "mix": 0, "open": 0, "unmix": 0, "close": 0, "remote": 0 }
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
    const rtp1 = new mocknode()
    const rtp2 = new mocknode()

    const listenport = 32443

    const rtpreceveivedmessages = []
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
    } )

    rtp1.setmessagehandler( "unmix", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
      rtp1.sendmessage( {
        "action": "unmix",
        "id": msg.id,
        "uuid": msg.uuid,
        "local": {
          "port": 10002,
          "address": "192.168.0.141"
        },
        "status": rtp1.ourstats
      } )
    } )

    rtp1.setmessagehandler( "remote", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
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
    } )

    rtp2.setmessagehandler( "unmix", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
      rtp2.sendmessage( {
        "action": "unmix",
        "id": msg.id,
        "uuid": msg.uuid,
        "local": {
          "port": 10002,
          "address": "192.168.0.141"
        },
        "status": rtp2.ourstats
      } )
    } )

    rtp2.setmessagehandler( "remote", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
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

    const p = await prtp.proxy.listen( undefined, "127.0.0.1", listenport )
    await rtp1.connect( listenport )
    await rtp2.connect( listenport )

    const channel1 = await prtp.openchannel( { "nodeinstance": rtp1.ourstats.instance } )
    const channel2 = await prtp.openchannel( { "nodeinstance": rtp2.ourstats.instance } )
    await channel1.mix( channel2 )
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1000 ) } )
    channel1.unmix()
    channel2.unmix()

    channel1.close()
    channel2.close()

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 10 ) } )

    for ( const msg of rtpreceveivedmessages ) actual[ msg.channel ] += 1
    expect( actual ).to.deep.equal( { "mix": 2, "open": 4, "unmix": 4, "close": 4, "remote": 2 } )

    /* Clean up */
    rtp1.destroy()
    rtp2.destroy()
    p.destroy()

  } )

  it( "2 node 1 channel on one, 2 channels other", async function() {

    this.timeout( 3000 )
    this.slow( 2500 )

    /*
      SIP                   rtp 1                          rtp 2
      open ------------------>                                        (1)
           <-----------------id:1                                     (2)
      open ------------------>                                        (3)
           <------------------id:2                                    (4)
      open -------------------------------------------------->        (5)
           <--------------------------------------------------id:3    (6)
      mix internally first:
          mix 1,2 ---------->                                         (7)
      mix converts to:
           open channel ----->                                        (8)
           <------------------id:4                                    (9)

           open channel ------------------------------------->        (10)
           <-------------------------------------------------id:5     (11)

           remote ----------->                                        (12)
           remote ------------------------------------------->        (13)

           mix 1,4 ---------->                                        (14)
           mix 2,5 ------------------------------------------>        (15)

      unmix (on both channels) converts to:
           unmix 1 ---------->                                        (16)
           unmix 2 ---------->                                        (17)
           unmix 4 ---------->                                        (18)

           unmix 3 ------------------------------------------>        (19)
           unmix 5 ------------------------------------------>        (20)

           close channel 4 -->                                        (21)
           close channel 5 ---------------------------------->        (22)
           close channel 1 -->                                        (23)
           close channel 2 -->                                        (24)
           close channel 3 ---------------------------------->        (25)
    */

    const actual = { "mix": 0, "open": 0, "unmix": 0, "close": 0, "remote": 0 }
    const rtp1 = mocknode.create()
    const rtp2 = mocknode.create()

    const listenport = 23455

    const rtpreceveivedmessages = []
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
    } )

    rtp1.setmessagehandler( "unmix", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
      rtp1.sendmessage( {
        "action": "unmix",
        "id": msg.id,
        "uuid": msg.uuid,
        "local": {
          "port": 10004,
          "address": "192.168.0.141"
        },
        "status": rtp1.ourstats
      } )
    } )

    rtp1.setmessagehandler( "remote", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
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
    } )

    rtp2.setmessagehandler( "unmix", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
      rtp2.sendmessage( {
        "action": "unmix",
        "id": msg.id,
        "uuid": msg.uuid,
        "local": {
          "port": 10004,
          "address": "192.168.0.141"
        },
        "status": rtp2.ourstats
      } )
    } )

    rtp2.setmessagehandler( "remote", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
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

    const p = await prtp.proxy.listen( undefined, "127.0.0.1", listenport )
    await rtp1.connect( listenport )
    await rtp2.connect( listenport )
    
    const channel1 = await prtp.openchannel( { "nodeinstance": rtp1.id } )
    const channel2 = await prtp.openchannel( { "nodeinstance": rtp2.id } )
    const channel3 = await prtp.openchannel( { "nodeinstance": rtp1.id } )
    await channel1.mix( channel3 )
    await channel1.mix( channel2 )
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1000 ) } )
    channel1.unmix()
    channel2.unmix()
    channel3.unmix()

    channel1.close()
    channel2.close()
    channel3.close()


    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 10 ) } )

    for ( const msg of rtpreceveivedmessages ) actual[ msg.channel ] += 1
    expect( actual ).to.deep.equal( { "mix": 3, "open": 5, "unmix": 5, "close": 5, "remote": 2 } )

    /* Clean up */
    rtp1.destroy()
    rtp2.destroy()
    p.destroy()
       

  } )

  it( "3 node 1 channel each, close main node", async function() {

    this.timeout( 3000 )
    this.slow( 2500 )

    /*
      SIP                   rtp 1                          rtp 2          rtp3
      open ------------------>                                                      (1)
           <-----------------id:1                                                   (2)
      open -------------------------------------------------->                      (3)
           <--------------------------------------------------id:2                  (4)
      open ---------------------------------------------------------------->        (5)
           <----------------------------------------------------------------id:3    (6)
      mix converts to:
           open channel ----->                                                      (7)
           <------------------id:4                                                  (8)
           open channel ------------------------------------->                      (9)
           <-------------------------------------------------id:5                   (10)
           open channel ----->                                                      (11)
           <------------------id:6                                                  (12)
           open channel --------------------------------------------------->        (13)
           <----------------------------------------------------------------id:7    (14)

           remote ----------->                                                      (15)
           remote ------------------------------------------->                      (16)
           remote --------------------------------------------------------->        (17)

           mix 1,4 ---------->                                                      (18)
           mix 2,5 ------------------------------------------>                      (19)
           mix 3,7 -------------------------------------------------------->        (20)

      unmix (on both channels) converts to:
           unmix 1 ---------->                                                      (21)
           unmix 4 ---------->                                                      (22)

           unmix 2 ------------------------------------------>                      (23)
           unmix 5 ------------------------------------------>                      (24)

           unmix 3 ------------------------------------------------------->         (25)
           unmix 6 ---------->                                                      (26)
           unmix 7 ------------------------------------------------------->         (27)


           close channel 4 -->                                                      (28)
           close channel 5 ---------------------------------->                      (29)
           close channel 6 -->                                                      (30)
           close channel 7 ----------------------------------------------->         (31)
           close channel 1 -->                                                      (32)
           close channel 2 ---------------------------------->                      (33)
           close channel 3 ----------------------------------------------->         (34)
    */
    const actual = { "mix": 0, "open": 0, "unmix": 0, "close": 0, "remote": 0 }
    const rtp1 = new mocknode()
    const rtp2 = new mocknode()
    const rtp3 = new mocknode()

    const listenport = 24553

    const rtpreceveivedmessages = []
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
    } )

    rtp1.setmessagehandler( "unmix", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
      rtp1.sendmessage( {
        "action": "unmix",
        "id": msg.id,
        "uuid": msg.uuid,
        "local": {
          "port": 10002,
          "address": "192.168.0.141"
        },
        "status": rtp1.ourstats
      } )
    } )

    rtp1.setmessagehandler( "remote", ( msg ) => {
      msg.node = "rtp1"
      rtpreceveivedmessages.push ( msg )
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
    } )

    rtp2.setmessagehandler( "unmix", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
      rtp2.sendmessage( {
        "action": "unmix",
        "id": msg.id,
        "uuid": msg.uuid,
        "local": {
          "port": 10002,
          "address": "192.168.0.141"
        },
        "status": rtp2.ourstats
      } )
    } )

    rtp2.setmessagehandler( "remote", ( msg ) => {
      msg.node = "rtp2"
      rtpreceveivedmessages.push( msg )
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

    rtp3.setmessagehandler( "open", ( msg ) => {
      msg.node = "rtp3"
      rtpreceveivedmessages.push ( msg )
      rtp3.sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": ""+ouruuid++,
        "local": {
          "port": 10002,
          "address": "192.168.0.141"
        },
        "status": rtp3.ourstats
      } )
    } )

    rtp3.setmessagehandler( "mix", ( msg ) => {
      msg.node = "rtp3"
      rtpreceveivedmessages.push ( msg )
    } )

    rtp3.setmessagehandler( "unmix", ( msg ) => {
      msg.node = "rtp3"
      rtpreceveivedmessages.push ( msg )
      rtp3.sendmessage( {
        "action": "unmix",
        "id": msg.id,
        "uuid": msg.uuid,
        "local": {
          "port": 10002,
          "address": "192.168.0.141"
        },
        "status": rtp3.ourstats
      } )
    } )

    rtp3.setmessagehandler( "remote", ( msg ) => {
      msg.node = "rtp3"
      rtpreceveivedmessages.push ( msg )
    } )

    rtp3.setmessagehandler( "close", ( msg ) => {
      msg.node = "rtp3"
      rtpreceveivedmessages.push ( msg )
      rtp3.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid,
      } )
    } )

    const p = await prtp.proxy.listen( undefined, "127.0.0.1", listenport )
    await rtp1.connect( listenport )
    await rtp2.connect( listenport )
    await rtp3.connect( listenport )
    
    const channel1 = await prtp.openchannel( { "nodeinstance": rtp1.id } )
    const channel2 = await prtp.openchannel( { "nodeinstance": rtp2.id } )
    const channel3 = await prtp.openchannel( { "nodeinstance": rtp3.id } )
    await channel1.mix( channel3 )
    await channel1.mix( channel2 )
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1000 ) } )
    // Close channel1, but the main node bridges should not be closed
    channel1.unmix()
    channel2.unmix()
    channel3.unmix()

    channel1.close()
    channel2.close()
    channel3.close()

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )

    for ( const msg of rtpreceveivedmessages ) actual[ msg.channel ] += 1
    expect( actual ).to.deep.equal( { "mix": 4, "open": 7, "unmix": 7, "close": 7, "remote": 4 } )


    /* Clean up */
    rtp1.destroy()
    rtp2.destroy()
    p.destroy()
      
  } )
} )