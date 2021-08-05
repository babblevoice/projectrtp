

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

#include <boost/enable_shared_from_this.hpp>
#include <boost/lockfree/stack.hpp>
#include <boost/smart_ptr/atomic_shared_ptr.hpp>

#include "boost/date_time/posix_time/posix_time.hpp"

/* CODECs */
#include <ilbc.h>
#include <spandsp.h>

#include "globals.h"
#include "projectrtpcodecx.h"
#include "projectrtppacket.h"
#include "projectrtpsoundsoup.h"
#include "controlclient.h"
#include "projectrtpsrtp.h"
#include "projectrtpchannelmux.h"

/* The number of packets we will keep in a buffer */
#define BUFFERPACKETCOUNT 20
/* The level we start dropping packets to clear backlog */
#define BUFFERPACKETCAP 15
#define BUFFERLOWDELAYCOUNT 5
#define BUFFERHIGHDELAYCOUNT 10 /* 200mS @ a ptime of 20mS */

/* The size of our message queue where we send info about new channels added
to the mixr to be added */
#define MIXQUEUESIZE 50

/* 1 in ... packet loss */
//#define SIMULATEDPACKETLOSSRATE 10



/*
# projectchannelmux

Generate our own tick. If we get up to multiple channels we don't want all to have a timer
firing - we want only one to mix all. A channel will maintain its own timer (when needed)
for things like playing a sound file (on single channels) or echo.
*/

/*!md
# projectrtpchannel
Purpose: RTP Channel - which represents RTP and RTCP. This is here we include our jitter buffer. We create a cyclic window to write data into and then read out of.

RTP on SIP channels should be able to switch between CODECS during a session so we have to make sure we have space for that.
*/


class projectrtpchannel :
  public std::enable_shared_from_this< projectrtpchannel >
{

public:
  friend projectchannelmux;
  projectrtpchannel( boost::asio::io_context &workercontext, boost::asio::io_context &iocontext, unsigned short port );
  ~projectrtpchannel( void );

  typedef std::shared_ptr< projectrtpchannel > pointer;
  static pointer create( boost::asio::io_context &workercontext, boost::asio::io_context &iocontext, unsigned short port );

  void open( std::string &id, std::string &uuid, controlclient::pointer );
  void close( void );
  void doclose( void );
  void go( void );

  unsigned short getport( void );

  void enabledtls( dtlssession::mode, std::string &fingerprint );

  void target( std::string &address, unsigned short port );
  void rfc2833( unsigned short pt );
  void play( stringptr newdef ) { this->newplaydef = newdef; }
  inline void echo( void ) { this->doecho = true; }

  void record( channelrecorder::pointer rec ) { this->newrecorders.push( rec ); }

  typedef std::vector< int > codeclist;
  bool audio( codeclist codecs );

  inline void direction( bool send, bool recv ) { this->send = send; this->recv = recv; }

  void writepacket( rtppacket * );
  void handlesend(
        const boost::system::error_code& error,
        std::size_t bytes_transferred);

  void handletick( const boost::system::error_code& error );

  bool canread( void ) { return this->reader; };
  bool canwrite( void ) { return this->writer; };

  bool isactive( void );

  bool mix( projectrtpchannel::pointer other );
  rtppacket *gettempoutbuf( uint32_t skipcount = 0 );

  void unmix( void );

  codeclist codecs;
  int selectedcodec;
  uint32_t ssrcout;
  uint32_t ssrcin;
  uint32_t tsout;
  uint16_t seqout;

  atomicrtppacketptr toolatertppacket;
  rtppacket rtpdata[ BUFFERPACKETCOUNT ];
  atomicrtppacketptr availablertpdata[ BUFFERPACKETCOUNT ];
  atomicrtppacketptr orderedrtpdata[ BUFFERPACKETCOUNT ];
  std::atomic_uint16_t orderedinminsn; /* sn = sequence number, min smallest we hold which is unprocessed - when it is processed we can forget about it */
  std::atomic_uint16_t orderedinmaxsn;
  std::atomic_uint16_t lastworkedonsn;
  std::atomic_uint16_t rtpbuffercount; /* keeps track of availble buffer items in availablertpdata */
  std::atomic_bool rtpbufferlock; /* spin lock to keep syncronised between reading and processing thread */


  unsigned char rtcpdata[ RTCPMAXLENGTH ];

  /* The out data is intended to be written by other channels (or functions), they can then be sent to other channels as well as our own end point  */
  rtppacket outrtpdata[ BUFFERPACKETCOUNT ];
  int rtpoutindex;

private:
  std::atomic_bool active;
  unsigned short port;
  unsigned short rfc2833pt;
  uint32_t lasttelephoneevent;

  /* id provided to us */
  std::string id;

  /* uuid we generate for this channel */
  std::string uuid;

  boost::asio::io_context &iocontext;
  boost::asio::io_context &workercontext;
  boost::asio::ip::udp::resolver resolver;

  boost::asio::ip::udp::socket rtpsocket;
  boost::asio::ip::udp::socket rtcpsocket;

  boost::asio::ip::udp::endpoint rtpsenderendpoint;
  boost::asio::ip::udp::endpoint confirmedrtpsenderendpoint;
  boost::asio::ip::udp::endpoint rtcpsenderendpoint;

  /* confirmation of where the other end of the RTP stream is */
  std::atomic_bool receivedrtp;
  bool targetconfirmed;

  bool reader;
  bool writer;
  void returnbuffer( rtppacket *buf );
  rtppacket* getbuffer( void );
  void readsomertp( void );
  void readsomertcp( void );

  /* Generally used as a pair */
  rtppacket *getrtpbottom( uint16_t highcount = BUFFERHIGHDELAYCOUNT, uint16_t lowcount = BUFFERLOWDELAYCOUNT );
  void incrrtpbottom( rtppacket *from );
  bool checkidlerecv( void );
  bool checkforoverrun( rtppacket *buf );
  void checkandfixoverrun( void );
  bool checkforunderrun( rtppacket *buf );
  void checkfornewrecorders( void );
  void writerecordings( void );

  void handlertcpdata( void );
  void handletargetresolve (
              boost::system::error_code e,
              boost::asio::ip::udp::resolver::iterator it );

  void displaybuffer( void );

  uint64_t receivedpkcount;
  uint64_t receivedpkskip;

  atomicmuxptr others;

  /* CODECs  */
  codecx outcodec;
  codecx incodec;

  soundsoup::pointer player;
  atomicstringptr newplaydef;

  std::atomic_bool doecho;
  boost::asio::steady_timer tick;
  controlclient::pointer control;

  std::atomic_uint16_t tickswithnortpcount;

  std::atomic_bool send;
  std::atomic_bool recv;

  /* Track hysteresis in our receive window - which allows for timing wiggles between our tick and when we receive */
  bool havedata;

  boost::lockfree::stack< boost::shared_ptr< channelrecorder > > newrecorders;
  std::list< boost::shared_ptr< channelrecorder > > recorders;

  /* for stats */
  uint64_t maxticktime;
  uint64_t totalticktime;
  uint64_t totaltickcount;

  /* DTLS Session */
  dtlssession::pointer rtpdtls;
  bool rtpdtlshandshakeing;

};

typedef std::deque<projectrtpchannel::pointer> rtpchannels;
typedef std::unordered_map<std::string, projectrtpchannel::pointer> activertpchannels;


#endif
