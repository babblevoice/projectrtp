#ifndef PROJECTRTPGLOBALS_H
#define PROJECTRTPGLOBALS_H

#include <boost/asio.hpp>
#include <string>
#include <memory>

#include <boost/smart_ptr/atomic_shared_ptr.hpp>

/* The number of bytes in a packet ( these figure are less some overhead G711 = 172*/
#define G711PAYLOADBYTES 160
#define G722PAYLOADBYTES 160
#define L16PAYLOADSAMPLES 160
#define L16NARROWBANDBYTES 320
#define L16WIDEBANDBYTES 640
#define ILBC20PAYLOADBYTES 38
#define ILBC30PAYLOADBYTES 50 /* not needed but for completness */

#define PCMUPAYLOADTYPE 0
#define PCMAPAYLOADTYPE 8
#define G722PAYLOADTYPE 9
#define ILBCPAYLOADTYPE 97
/* Only use this value for internal use and must not clash with the types above */
#define L168KPAYLOADTYPE 11
#define L1616KPAYLOADTYPE 12

/* Need to double check max RTP length with variable length header - there
could be a larger length with our CODECs */

/* this maybe breached if a stupid number of csrc count is high */
#define RTPMAXLENGTH 200
#define L16MAXLENGTH ( RTPMAXLENGTH * 2 )
#define RTCPMAXLENGTH 200


extern std::string mediachroot;

/* Switch to boost for now as atomic shared pointer in std doesn't compile
std::atomic_store( stringptr
ddn't complain about the tagerget object not being atomic also!
*/
typedef boost::shared_ptr< std::string > stringptr;
typedef boost::atomic_shared_ptr< std::string > atomicstringptr;

#endif /* PROJECTRTPGLOBALS_H */
