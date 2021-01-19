

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

#include <boost/lockfree/stack.hpp>

/* CODECs */
#include <ilbc.h>
#include <spandsp.h>

#include "globals.h"
#include "projectrtpcodecx.h"
#include "projectrtppacket.h"
#include "projectrtpsoundsoup.h"
#include "controlclient.h"

/* The number of packets we will keep in a buffer */
#define BUFFERPACKETCOUNT 20
#define BUFFERDELAYCOUNT 10
#define MIXQUEUESIZE 50

/* 1 in ... packet loss */
//#define SIMULATEDPACKETLOSSRATE 10


/*!md
# projectrtpchannel
Purpose: RTP Channel - which represents RTP and RTCP. This is here we include our jitter buffer. We create a cyclic window to write data into and then read out of.

RTP on SIP channels should be able to switch between CODECS during a session so we have to make sure we have space for that.
*/


class projectrtpchannel :
  public std::enable_shared_from_this< projectrtpchannel >
{

public:
  projectrtpchannel( boost::asio::io_context &iocontext, unsigned short port );
  ~projectrtpchannel( void );

  typedef std::shared_ptr< projectrtpchannel > pointer;
  static pointer create( boost::asio::io_context &iocontext, unsigned short port );

  void open( std::string &id, std::string &uuid, controlclient::pointer );
  void close( void );
  void doclose( void );

  unsigned short getport( void );

  void target( std::string &address, unsigned short port );
  void rfc2833( unsigned short pt );
  void play( stringptr newdef ) { std::atomic_store( &this->newplaydef, newdef ); }
  inline void echo( void ) { this->doecho = true; }

  typedef std::vector< int > codeclist;
  bool audio( codeclist codecs );

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

  rtppacket rtpdata[ BUFFERPACKETCOUNT ];
  rtppacket *orderedrtpdata[ BUFFERPACKETCOUNT ];
  std::atomic_uint16_t orderedinminsn; /* sn = sequence number, min smallest we hold which is unprocessed - when it is processed we can forget about it */
  std::atomic_uint16_t orderedinmaxsn;
  std::atomic_uint16_t orderedinbottom; /* points to our min sn packet */
  std::atomic_uint16_t lastworkedonsn;

  unsigned char rtcpdata[ RTCPMAXLENGTH ];
  int rtpindexoldest;
  int rtpindexin;

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
  void readsomertp( void );
  void readsomertcp( void );

  bool handlertpdata( void );
  void processrtpdata( rtppacket *src, uint32_t skipcount );
  void handlertcpdata( void );
  void handletargetresolve (
              boost::system::error_code e,
              boost::asio::ip::udp::resolver::iterator it );


  void checkfornewmixes( void );
  uint64_t receivedpkcount;
  uint64_t receivedpkskip;

  typedef std::list< projectrtpchannel::pointer > projectrtpchannellist;
  typedef boost::shared_ptr< projectrtpchannellist > projectrtpchannellistptr;
  projectrtpchannellistptr others;

  /* CODECs  */
  codecx codecworker;

  soundsoup::pointer player;
  stringptr newplaydef;

  std::atomic_bool doecho;

  boost::lockfree::stack< projectrtpchannel::pointer > mixqueue;
  boost::asio::steady_timer tick;

  controlclient::pointer control;
};

typedef std::deque<projectrtpchannel::pointer> rtpchannels;
typedef std::unordered_map<std::string, projectrtpchannel::pointer> activertpchannels;


#endif
