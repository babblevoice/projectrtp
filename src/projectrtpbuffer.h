
#ifndef PROJECTRTPBUFFER_H
#define PROJECTRTPBUFFER_H


#include <memory>
#include <vector>
#include <queue>

#include "projectrtppacket.h"

/* Defaults */
/* 
  The number of packets we will keep in a buffer 
  MUST BE POWER OF 2 or the mod won't work on the 
  circular buffer when wrapping around the uint16.
*/
#define BUFFERPACKETCOUNT 32
/* 
  How many packets we receive before we start taking packets out of the buffer 
  If we fill, Delay = ( BUFFERPACKETCOUNT - BUFFERPACKETCAP ) * 20mS 
*/
#define BUFFERPACKETCAP 10  /* 200mS @ a ptime of 20mS */

/*
My thoughts on buffers. We reorder as we might want to use the data
and will will probably hepl the other end if we are proxying.

If the buffer is too big we can introduce delay. If it is too short
then we might introduce losses.
*/

class rtpbuffer :
  public std::enable_shared_from_this< rtpbuffer > {

public:

  rtpbuffer( int count, int waterlevel );
  ~rtpbuffer();

  rtpbuffer( const rtpbuffer& ) = delete;              // copy ctor
  rtpbuffer( rtpbuffer&& ) = delete;                   // move ctor
  rtpbuffer& operator=( const rtpbuffer& ) = delete;   // copy assignment
  rtpbuffer& operator=( rtpbuffer&& ) = delete;        // move assignment

  typedef std::shared_ptr< rtpbuffer > pointer;
  static pointer create( int count = BUFFERPACKETCOUNT, /* size of the array to store packets */
                         int waterlevel = BUFFERPACKETCAP /* the level we build up before allowing a read */ );

  rtppacket* peek( void );
  rtppacket* poppeeked( void );
  rtppacket* pop( void );
  void push( void );
  rtppacket* reserve( void );
  uint64_t getdropped( void ) { return this->dropped; }
  uint64_t getpushed( void ) { return this->pushed; }
  uint64_t getpopped( void ) { return this->popped; }
  uint64_t getbadsn( void ) { return this->badsn; }
  uint16_t getoutsn( void ) { return this->outsn; }

  int size( void ) { return this->buffercount; }

  typedef std::vector< rtppacket > rtppackets;
  typedef std::vector< rtppacket* > rtppacketptrs;
  typedef std::queue< rtppacket* > qrtppacketptrs;

private:

  /*
    buffer - used as the actual buffer for data
    availablertpdata - all of the items in buffer which are free to use
    orderedrtpdata - we have received data and verified it ok - so it needs
                     processing - items are in order based on sn
    reserved - after we reserve - this is where it is held until filled
  */
  rtppackets buffer;
  qrtppacketptrs availablertpdata;
  rtppacketptrs orderedrtpdata;
  rtppacket *reserved;
  /* Used to ensure we maintain consistency between a peek and a pop */
  rtppacket *peekedpopped;

  int buffercount;
  int waterlevel;
  uint16_t outsn;
  uint64_t dropped;
  uint64_t pushed;
  uint64_t popped;
  uint64_t badsn;
};

#ifdef TESTSUITE
void testrtpbuffer( void );
#endif

#ifdef NODE_MODULE

#include <node_api.h>
void initrtpbuffer( napi_env env, napi_value &result );

#endif /* NODE_MODULE */


#endif /* PROJECTRTPBUFFER_H */
