

#ifndef PROJECTRTPCHANNEL_H
#define PROJECTRTPCHANNEL_H

#include <memory>
#include <atomic>

#include <boost/asio.hpp>
#include <boost/asio/ip/udp.hpp>

#include <stdint.h>
#include <arpa/inet.h>

#include <string>
#include <list>
#include <vector>
#include <unordered_map>

#include <boost/date_time/posix_time/posix_time.hpp>

/* CODECs */
#include <ilbc.h>
#include <spandsp.h>


#include "globals.h"
#include "projectrtpbuffer.h"
#include "projectrtpcodecx.h"
#include "projectrtppacket.h"
#include "projectrtpsoundsoup.h"
#include "projectrtpchannelrecorder.h"
#include "projectrtpsrtp.h"

class projectrtpchannel;
class projectchannelmux;
#include "projectrtpchannelmux.h"

/* Must be to the power 2 */
#define OUTBUFFERPACKETCOUNT 16


/*
# projectrtpchannel
Purpose: RTP Channel - which represents RTP and RTCP. This is here we include our jitter buffer. We create a cyclic window to write data into and then read out of.

RTP on SIP channels should be able to switch between CODECS during a session so we have to make sure we have space for that.
*/

typedef std::shared_ptr< projectchannelmux > projectchannelmuxptr;

typedef std::list< channelrecorder::pointer > chanrecptrlist;

class projectrtpchannel :
  public std::enable_shared_from_this< projectrtpchannel > {

public:
  friend projectchannelmux;
  projectrtpchannel( unsigned short port );
  ~projectrtpchannel( void );

  projectrtpchannel( const projectrtpchannel& ) = delete;              // copy ctor
  projectrtpchannel( projectrtpchannel&& ) = delete;                   // move ctor
  projectrtpchannel& operator=( const projectrtpchannel& ) = delete;   // copy assignment
  projectrtpchannel& operator=( projectrtpchannel&& ) = delete;        // move assignment

  typedef std::shared_ptr< projectrtpchannel > pointer;
  static pointer create( unsigned short port );

  void remote( std::string address,
               unsigned short port,
               uint32_t codec,
               dtlssession::mode,
               std::string fingerprint );

  uint32_t requestopen( void );
  std::atomic_bool _requestclose;
  void requestclose( std::string reason );
  std::string closereason;
  void requestecho( bool e = true );

  void doremote( void );
  void doclose( void );
  void doopen( void );

  unsigned short getport( void );

  void requestplay( soundsoup::pointer newdef );
  void requestrecord( channelrecorder::pointer rec );
  inline void echo( void ) { this->doecho = true; }
  inline void direction( bool send, bool recv ) { this->send = send; this->recv = recv; }
  void writepacket( rtppacket * );
  void handlesend(
        const boost::system::error_code& error,
        std::size_t bytes_transferred);

  void handletick( const boost::system::error_code& error );
  bool isactive( void );

  bool mix( projectrtpchannel::pointer other );
  bool unmix( void );
  void dtmf( std::string digits );
  rtppacket *gettempoutbuf( void );

  uint32_t codec;
  uint32_t ssrcin;
  uint32_t ssrcout;
  uint32_t tsout;
  uint16_t snout;

  /* do we send, do we receive */
  std::atomic_bool send;
  std::atomic_bool recv;

  /* for stats */
  std::atomic_uint64_t receivedpkcount;
  std::atomic_uint64_t receivedpkskip;
  std::atomic_uint64_t maxticktime;
  std::atomic_uint64_t totalticktime;
  std::atomic_uint64_t totaltickcount;
  std::atomic_uint16_t tickswithnortpcount;

  std::atomic_uint64_t outpkcount;
  std::atomic_uint64_t outpkskipcount;

  /* buffer and spin lock for in traffic */
  rtpbuffer::pointer inbuff;
  std::atomic_bool rtpbufferlock;

  unsigned char rtcpdata[ RTCPMAXLENGTH ];

  /* The out data is intended to be written by other channels
     (or functions), they can then be sent to other channels
     as well as our own end point  */
  rtppacket outrtpdata[ OUTBUFFERPACKETCOUNT ];
  std::atomic_uint16_t rtpoutindex;

  /* ice */
  std::string icelocalpwd;
  std::string iceremotepwd;

#ifdef NODE_MODULE
  napi_threadsafe_function cb;
#endif

private:
  std::atomic_bool active;
  unsigned short port;
  unsigned short rfc2833pt;
  uint32_t lasttelephoneevent;

  boost::asio::ip::udp::resolver resolver;

  boost::asio::ip::udp::socket rtpsocket;
  boost::asio::ip::udp::socket rtcpsocket;

  boost::asio::ip::udp::endpoint rtpsenderendpoint;
  boost::asio::ip::udp::endpoint confirmedrtpsenderendpoint;
  boost::asio::ip::udp::endpoint rtcpsenderendpoint;

  /* confirmation of where the other end of the RTP stream is */
  std::atomic_bool receivedrtp;
  std::atomic_bool remoteconfirmed;

  void readsomertp( void );
  void readsomertcp( void );
  void incrtsout( void );

  bool checkidlerecv( void );
  void checkfornewrecorders( void );
  void removeoldrecorders( void );
  void writerecordings( void );

  bool checkfordtmf( rtppacket *src );
  void senddtmf( void );

  void handlertcpdata( void );
  void handleremoteresolve (
              boost::system::error_code e,
              boost::asio::ip::udp::resolver::iterator it );

  bool dtlsnegotiate( void );
  void setnexttick( void );
  void startticktimer( void );
  void endticktimer( void );

  bool handlestun( uint8_t *pk, size_t len );

  static bool recordercompleted( const channelrecorder::pointer& value );

  std::atomic_bool mixerlock;
  projectchannelmuxptr mixer;
  std::atomic_bool mixing;
  std::atomic_bool removemixer;

  /* CODECs  */
  codecx outcodec;
  codecx incodec;

  soundsoup::pointer player;
  soundsoup::pointer newplaydef;
  std::atomic_bool newplaylock;

  std::atomic_bool doecho;
  boost::asio::steady_timer tick;
  std::chrono::high_resolution_clock::time_point nexttick;

  chanrecptrlist newrecorders;
  std::atomic_bool newrecorderslock;
  chanrecptrlist recorders;

  /* DTLS Session */
  dtlssession::pointer rtpdtls;
  std::atomic_bool rtpdtlslock;

  std::string remoteaddress;
  unsigned short remoteport;

  /* outbound DTMF */
  std::string queueddigits;
  std::atomic_bool queuddigitslock;
  uint16_t lastdtmfsn;

  boost::posix_time::ptime tickstarttime;


  uint8_t stuntmpout[ 300 ];
};

typedef std::deque<projectrtpchannel::pointer> rtpchannels;
typedef std::unordered_map<std::string, projectrtpchannel::pointer> activertpchannels;

#ifdef NODE_MODULE
#include <node_api.h>

void initrtpchannel( napi_env env, napi_value &result );
void getchannelstats( napi_env env, napi_value &result );

class jschannelevent {
public:
  jschannelevent( projectrtpchannel::pointer p, std::string event, std::string arg1 = "", std::string arg2 = "" ):
    event( event ), arg1( arg1 ), arg2( arg2 ), p( p ) {}

  std::string event;
  std::string arg1;
  std::string arg2;

  projectrtpchannel::pointer p;
};

/* The one function used by our channel */
void postdatabacktojsfromthread( projectrtpchannel::pointer p, std::string event, std::string arg1 = "", std::string arg2 = "" );

#else
#define postdatabacktojsfromthread( ... )
#endif /* NODE_MODULE */

#endif
