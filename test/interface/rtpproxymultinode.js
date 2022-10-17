

const expect = require( "chai" ).expect
const prtp = require( "../../index.js" ).projectrtp
const mocknode = require( "../mock/mocknode" )

describe( "rtpproxy multi node", function() {

  it( `2 node simple mix`, async function() {
    let expected = { "mix": 2, "open": 4, "unmix": 4, "close": 4, "remote": 2 }
    let actual = { "mix": 0, "open": 0, "unmix": 0, "close": 0, "remote": 0 }
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

    let p = await prtp.proxy.listen( undefined, "127.0.0.1", listenport )
    rtp1.connect( listenport )
    rtp2.connect( listenport )
    await p.waitfornewconnection()
    let channel1 = await prtp.openchannel( { "nodeinstance": rtp1.ourstats.instance } )
    let channel2 = await prtp.openchannel( { "nodeinstance": rtp2.ourstats.instance } )
    await channel1.mix( channel2 )
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1000 ) } )
    channel1.unmix()
    channel2.unmix()

    channel1.close()
    channel2.close()

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 10 ) } )

    for ( let msg of rtpreceveivedmessages )
    {
      actual[ msg.channel ] += 1
    }
    for ( let action in expected )
    {
      expect( expected[ action ] ).to.equal( actual[ action ] )
    }
    /*for ( let msg1 in rtpreceveivedmessages )
    {
      console.log( msg1, " ", rtpreceveivedmessages[msg1] )
    }
     Steps 1-4 
    expect( rtpreceveivedmessages[ 0 ].channel ).to.equal( "open" )
    expect( rtpreceveivedmessages[ 0 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 1 ].channel ).to.equal( "open" )
    expect( rtpreceveivedmessages[ 1 ].node ).to.equal( "rtp2" )

     Steps 5-8 
    expect( rtpreceveivedmessages[ 2 ].channel ).to.equal( "open" )
    expect( rtpreceveivedmessages[ 2 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 3 ].channel ).to.equal( "open" )
    expect( rtpreceveivedmessages[ 3 ].node ).to.equal( "rtp2" )
 
     Steps 9, 10 
    expect( rtpreceveivedmessages[ 4 ].channel ).to.equal( "remote" )
    expect( rtpreceveivedmessages[ 4 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 4 ].remote.port ).to.be.an( "number" )
    expect( rtpreceveivedmessages[ 4 ].remote.address ).to.be.an( "string" )
    expect( rtpreceveivedmessages[ 5 ].channel ).to.equal( "remote" )
    expect( rtpreceveivedmessages[ 5 ].node ).to.equal( "rtp2" )
    expect( rtpreceveivedmessages[ 5 ].remote.port ).to.be.an( "number" )
    expect( rtpreceveivedmessages[ 5 ].remote.address ).to.be.an( "string" )


     Steps 11, 12 - this is sometimes out of order
    expect( rtpreceveivedmessages[ 6 ].channel ).to.equal( "mix" )
    expect( rtpreceveivedmessages[ 6 ].node ).to.equal( "rtp2" )
    expect( rtpreceveivedmessages[ 7 ].channel ).to.equal( "mix" )
    expect( rtpreceveivedmessages[ 7 ].node ).to.equal( "rtp1" )

     Steps 13, 14
    expect( rtpreceveivedmessages[ 8 ].channel ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 8 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 8 ] ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 8 ] ).to.have.property( "id" ).that.is.a( "string" )
    expect( rtpreceveivedmessages[ 8 ] ).to.have.property( "uuid" ).that.is.a( "string" )
    expect( rtpreceveivedmessages[ 9 ].channel ).to.equal( "close" )
    expect( rtpreceveivedmessages[ 9 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 10 ].channel ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 10 ].node ).to.equal( "rtp2" )
    expect( rtpreceveivedmessages[ 10 ] ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 10 ] ).to.have.property( "id" ).that.is.a( "string" )
    expect( rtpreceveivedmessages[ 10 ] ).to.have.property( "uuid" ).that.is.a( "string" )
    expect( rtpreceveivedmessages[ 11 ].channel ).to.equal( "close" )
    expect( rtpreceveivedmessages[ 11 ].node ).to.equal( "rtp2" )

    expect( rtpreceveivedmessages[ 10 ].channel ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 10 ].node ).to.equal( "rtp2" )
    expect( rtpreceveivedmessages[ 10 ] ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
    expect( rtpreceveivedmessages[ 10 ] ).to.have.property( "id" ).that.is.a( "string" )
    expect( rtpreceveivedmessages[ 10 ] ).to.have.property( "uuid" ).that.is.a( "string" )

     Steps 17, 18 
    expect( rtpreceveivedmessages[ 9 ].channel ).to.equal( "close" )
    expect( rtpreceveivedmessages[ 9 ].node ).to.equal( "rtp1" )
    expect( rtpreceveivedmessages[ 11 ].channel ).to.equal( "close" )
    expect( rtpreceveivedmessages[ 11 ].node ).to.equal( "rtp2" ) */

    /* Clean up */
    rtp1.destroy()
    rtp2.destroy()
    p.destroy()

  } )

  it( `2 node 1 channel on one, 2 channels other`, async function() {
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

    let expected = { "mix": 3, "open": 5, "unmix": 5, "close": 5, "remote": 2 }
    let actual = { "mix": 0, "open": 0, "unmix": 0, "close": 0, "remote": 0 }
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

    let p = await prtp.proxy.listen( undefined, "127.0.0.1", listenport )
    rtp1.connect( listenport )
    rtp2.connect( listenport )
    await p.waitfornewconnection()
    
    let channel1 = await prtp.openchannel( { "nodeinstance": rtp1.id } )
    let channel2 = await prtp.openchannel( { "nodeinstance": rtp2.id } )
    let channel3 = await prtp.openchannel( { "nodeinstance": rtp1.id } )
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

    for ( let msg of rtpreceveivedmessages )
    {
      actual[ msg.channel ] += 1
    }
    for ( let action in expected )
    {
      expect( expected[ action ] ).to.equal( actual[ action ] )
    }
    /* Clean up */
    rtp1.destroy()
    rtp2.destroy()
    p.destroy()
       

  } )

  it( `2 node 2 channels on one, 2 channels other`, async function() {
/*
      SIP                   rtp 1                          rtp 2
      open ------------------>                                        (1)
           <-----------------id:1                                     (2)
      open -------------------------------------------------->        (3)
           <--------------------------------------------------id:2    (4)
      open -------------------------------------------------->        (5)
           <--------------------------------------------------id:5    (6)
      open -------------------------------------------------->        (7)
           <--------------------------------------------------id:6    (8)
      mix internally first:
          mix 1,5 ---------->                                         (9)
          mix 2,6 ---------->                                         (10)
      mix converts to:
           open channel ----->                                        (11)
           <------------------id:3                                    (12)

           open channel ------------------------------------->        (13)
           <-------------------------------------------------id:4     (14)

           remote ----------->                                        (15)
           remote ------------------------------------------->        (16)

           mix 1,3 ---------->                                        (17)
           mix 2,4 ------------------------------------------>        (18)

      unmix (on both channels) converts to:
           unmix 1 ---------->                                        (19)
           unmix 3 ---------->                                        (20)
           unmix 5 ---------->                                        (21)
           unmix 6 ---------->                                        (22)

           unmix 2 ------------------------------------------>        (23)
           unmix 4 ------------------------------------------>        (24)

           close channel 3 -->                                        (25)
           close channel 4 ---------------------------------->        (26)
           close channel 1 -->                                        (27)
           close channel 5 -->                                        (28)
           close channel 2 ---------------------------------->        (29)
           close channel 6 ---------------------------------->        (30)
    */
      let expected = { "mix": 4, "open": 6, "unmix": 6, "close": 6, "remote": 2 }
      let actual = { "mix": 0, "open": 0, "unmix": 0, "close": 0, "remote": 0 }
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
  
      let p = await prtp.proxy.listen( undefined, "127.0.0.1", listenport )
      rtp1.connect( listenport )
      rtp2.connect( listenport )
      await p.waitfornewconnection()
      
      let channel1 = await prtp.openchannel( { "nodeinstance": rtp1.id } )
      let channel2 = await prtp.openchannel( { "nodeinstance": rtp2.id } )
      let channel3 = await prtp.openchannel( { "nodeinstance": rtp1.id } )
      let channel4 = await prtp.openchannel( { "nodeinstance": rtp2.id } )
      await channel1.mix( channel3 )
      await channel2.mix( channel4 )
      await channel1.mix( channel2 )
      await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1000 ) } )
      channel1.unmix()
      channel2.unmix()
      channel3.unmix()
      channel4.unmix()
  
      channel1.close()
      channel2.close()
      channel3.close()
      channel4.close()
  
      await new Promise( ( resolve ) => { setTimeout( () => resolve(), 10 ) } )

    for ( let msg of rtpreceveivedmessages )
    {
      actual[ msg.channel ] += 1
    }
    for ( let action in expected )
    {
      expect( expected[ action ] ).to.equal( actual[ action ] )
    }
  
      /* Clean up */
      rtp1.destroy()
      rtp2.destroy()
      p.destroy()
              
       
  } )

  it( `3 node 1 channel each, close main node`, async function() {
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
    let expected = { "mix": 4, "open": 7, "unmix": 7, "close": 7, "remote": 4 }
    let actual = { "mix": 0, "open": 0, "unmix": 0, "close": 0, "remote": 0 }
    let rtp1 = new mocknode()
    let rtp2 = new mocknode()
    let rtp3 = new mocknode()

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

    let p = await prtp.proxy.listen( undefined, "127.0.0.1", listenport )
    rtp1.connect( listenport )
    rtp2.connect( listenport )
    rtp3.connect( listenport )
    await p.waitfornewconnection()
    
    let channel1 = await prtp.openchannel( { "nodeinstance": rtp1.id } )
    let channel2 = await prtp.openchannel( { "nodeinstance": rtp2.id } )
    let channel3 = await prtp.openchannel( { "nodeinstance": rtp3.id } )
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

    for ( let msg of rtpreceveivedmessages )
    {
      actual[ msg.channel ] += 1
    }
    for ( let action in expected )
    {
      expect( expected[ action ] ).to.equal( actual[ action ] )
    }

    /* Clean up */
    rtp1.destroy()
    rtp2.destroy()
    p.destroy()
      
   } )
       
} )