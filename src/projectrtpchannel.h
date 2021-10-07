

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

#include <node_api.h>

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

/* The number of packets we will keep in a buffer */
#define BUFFERPACKETCOUNT 20
/* The level we start dropping packets to clear backlog */
#define BUFFERPACKETCAP 10  /* 200mS @ a ptime of 20mS */

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

  void requestopen( std::string address, unsigned short port, uint32_t codec );
  std::atomic_bool requestclose;
  std::string closereason;
  void requestecho( bool e = true );

  void dotarget( void );
  void doclose( void );
  void doopen( void );

  unsigned short getport( void );

  void enabledtls( dtlssession::mode, std::string &fingerprint );

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
  rtppacket *gettempoutbuf( void );

  void unmix( void );

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

  /* buffer and spin lock for in traffic */
  rtpbuffer::pointer inbuff;
  std::atomic_bool rtpbufferlock;

  unsigned char rtcpdata[ RTCPMAXLENGTH ];

  /* The out data is intended to be written by other channels
     (or functions), they can then be sent to other channels
     as well as our own end point  */
  rtppacket outrtpdata[ OUTBUFFERPACKETCOUNT ];
  std::atomic_uint16_t rtpoutindex;

  napi_value jsthis;
  napi_threadsafe_function cb;

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
  std::atomic_bool targetconfirmed;

  void readsomertp( void );
  void readsomertcp( void );
  void incrtsout( void );

  bool checkidlerecv( void );
  void checkfornewrecorders( void );
  void removeoldrecorders( void );
  void writerecordings( void );

  bool checkfordtmf( rtppacket *src );

  void handlertcpdata( void );
  void handletargetresolve (
              boost::system::error_code e,
              boost::asio::ip::udp::resolver::iterator it );

  std::atomic_bool mixerlock;
  projectchannelmuxptr mixer;
  std::atomic_bool mixing;

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
  bool rtpdtlshandshakeing;

  std::string targetaddress;
  unsigned short targetport;

};

typedef std::deque<projectrtpchannel::pointer> rtpchannels;
typedef std::unordered_map<std::string, projectrtpchannel::pointer> activertpchannels;


void initrtpchannel( napi_env env, napi_value &result );

class jschannelevent {
public:
  jschannelevent( projectrtpchannel::pointer p, std::string event, std::string arg1 = "", std::string arg2 = "" ):
    event( event ), arg1( arg1 ), arg2( arg2 ), p( p ) {}

  std::string event;
  std::string arg1;
  std::string arg2;

  projectrtpchannel::pointer p;
};

#endif
